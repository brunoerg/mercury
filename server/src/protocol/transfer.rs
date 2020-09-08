//! StateEntity Transfer
//!
//! StateEntity Transfer protocol trait and implementation.

pub use super::super::Result;
extern crate shared_lib;
use shared_lib::{state_chain::*, structs::*, ecies, ecies::{WalletDecryptable}};
use super::transfer_batch::transfer_batch_is_ended;

use crate::error::SEError;
use crate::Database;
use crate::{server::StateChainEntity, storage::Storage};

use bitcoin::Transaction;
use cfg_if::cfg_if;
use curv::{
    elliptic::curves::traits::{ECPoint, ECScalar},
    {BigInt, FE, GE},
};
use rocket::State;
use rocket_contrib::json::Json;
use uuid::Uuid;
use std::str::FromStr;

cfg_if! {
    if #[cfg(any(test,feature="mockdb"))]{
        use crate::MockDatabase as DB;
        type SCE = StateChainEntity::<DB>;
    } else {
        use crate::PGDatabase as DB;
        type SCE = StateChainEntity::<DB>;
    }
}

/// Struct holds data when transfer is complete but not yet finalized
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransferFinalizeData {
    pub new_shared_key_id: Uuid,
    pub state_chain_id: Uuid,
    pub state_chain_sig: StateChainSig,
    pub s2: FE,
    pub new_tx_backup: Transaction,
    pub batch_data: Option<BatchData>,
}

/// StateChain Transfer protocol trait
pub trait Transfer {
    /// API: Initiliase transfer protocol:
    ///     - Authorisation of Owner and DoS protection
    ///     - Validate transfer parameters
    ///     - Store transfer parameters
    fn transfer_sender(&self, transfer_msg1: TransferMsg1) -> Result<TransferMsg2>;

    /// API: Transfer shared wallet to new Owner:
    ///     - Check new Owner's state chain is correct
    ///     - Perform 2P-ECDSA key rotation
    ///     - Return new public shared key S2
    fn transfer_receiver(&self, transfer_msg4: TransferMsg4) -> Result<TransferMsg5>;

    /// Update DB and SMT after successful transfer.
    /// This function is called immediately in the regular transfer case or after confirmation of atomic
    /// transfers completion in the batch transfer case.
    fn transfer_finalize(&self, finalized_data: &TransferFinalizeData) -> Result<()>;
}

impl Transfer for SCE {
    fn transfer_sender(&self, transfer_msg1: TransferMsg1) -> Result<TransferMsg2> {
        let user_id = transfer_msg1.shared_key_id;
        self.check_user_auth(&user_id)?;

        info!("TRANSFER: Sender Side. Shared Key ID: {}", user_id);

        // Get state_chain id
        let state_chain_id = self.database.get_statechain_id(user_id)?;

        // Check if transfer has already been completed (but not finalized)
        if self.database.transfer_is_completed(state_chain_id) {
            return Err(SEError::Generic(String::from(
                "Transfer already completed. Waiting for finalize.",
            )));
        }

        // Check if state chain is owned by user and not locked
        let sco = self.database.get_statechain_owner(state_chain_id)?;

        is_locked(sco.locked_until)?;
        if sco.owner_id != user_id {
            return Err(SEError::Generic(format!(
                "State Chain not owned by User ID: {}.",
                user_id
            )));
        }

        // Generate x1
        let x1: FE = ECScalar::new_random();
        let x1_ser = FESer::from_fe(&x1);

        self.database
            .create_transfer(&state_chain_id, &transfer_msg1.state_chain_sig, &x1)?;

        info!(
            "TRANSFER: Sender side complete. Previous shared key ID: {}. State Chain ID: {}",
            user_id.to_string(),
            state_chain_id
        );
        debug!("TRANSFER: Sender side complete. State Chain ID: {}. State Chain Signature: {:?}. x1: {:?}.", state_chain_id, transfer_msg1.state_chain_sig, x1);

        // encrypt x1 with Senders proof key
        let proof_key = match ecies::PublicKey::from_str(&self.database.get_proof_key(user_id)?){
            Ok(k) => k,
            Err(e) => return Err(SEError::SharedLibError(format!("error deserialising proof key: {}", e))),
        };

        let mut msg2 = TransferMsg2 {
            x1: x1_ser,
            proof_key
        };

        match msg2.encrypt() {
            Ok(_) => (),
            Err(e) => return Err(SEError::SharedLibError(format!("{}",e))),
        };

        let msg2 = msg2;

        Ok(msg2)
    }

    fn transfer_receiver(&self, transfer_msg4: TransferMsg4) -> Result<TransferMsg5> {
        let user_id = transfer_msg4.shared_key_id;
        let state_chain_id = transfer_msg4.state_chain_id;

        info!("TRANSFER: Receiver side. Shared Key ID: {}", user_id);

        // Get Transfer Data for state_chain_id
        let td = self.database.get_transfer_data(state_chain_id)?;

        // Ensure state_chain_sigs are the same
        if td.state_chain_sig != transfer_msg4.state_chain_sig.to_owned() {
            return Err(SEError::Generic(format!(
                "State chain siganture provided does not match state chain at id {}",
                state_chain_id
            )));
        }

        let kp = self.database.get_ecdsa_keypair(user_id)?;


        // let x1 = transfer_data.x1;
        let t2 = transfer_msg4.t2;

        let s1 = kp.party_1_private.get_private_key();

        // Note:
        //  s2 = o1*o2_inv*s1
        //  t2 = o1*x1*o2_inv
        let s2 = t2 * (td.x1.invert()) * s1;

        // Check s2 is valid for Lindell protocol (s2<q/3)
        let sk_bigint = s2.to_big_int();
        let q_third = FE::q();
        if sk_bigint >= q_third.div_floor(&BigInt::from(3)) {
            return Err(SEError::Generic(format!("Invalid o2, try again.")));
        }

        let g: GE = ECPoint::generator();
        let s2_pub: GE = g * s2;

        let p1_pub = kp.party_2_public * s1;
        let p2_pub = transfer_msg4.o2_pub * s2;

        // Check P1 = o1_pub*s1 === p2 = o2_pub*s2
        if p1_pub != p2_pub {
            error!("TRANSFER: Protocol failed. P1 != P2.");
            return Err(SEError::Generic(String::from(
                "Transfer protocol error: P1 != P2",
            )));
        }

        // Create user ID for new UserSession (receiver of transfer)
        let new_shared_key_id = Uuid::new_v4();

        let finalized_data = TransferFinalizeData {
            new_shared_key_id: new_shared_key_id.clone(),
            state_chain_id: state_chain_id.clone(),
            state_chain_sig: td.state_chain_sig,
            s2,
            new_tx_backup: transfer_msg4.tx_backup.clone(),
            batch_data: transfer_msg4.batch_data.clone(),
        };

        // If batch transfer then mark StateChain as complete and store finalized data in TransferBatch table.
        // This is so the transfers can be finalized when all transfers in the batch are complete.
        if transfer_msg4.batch_data.is_some() {
            let batch_id = transfer_msg4.batch_data.clone().unwrap().id;
            info!(
                "TRANSFER: Transfer as part of batch {}. State Chain ID: {}",
                batch_id, state_chain_id
            );
            // Get TransferBatch data
            let mut tbd = self.database.get_finalize_batch_data(batch_id)?;

            // Ensure batch transfer is still active
            if transfer_batch_is_ended(tbd.start_time, self.config.batch_lifetime as i64) {
                return Err(SEError::Generic(String::from(
                    "Transfer batch ended. Too late to complete transfer.",
                )));
            }

            tbd.state_chains.insert(state_chain_id.clone(), true);
            tbd.finalized_data_vec.push(finalized_data.clone());

            self.database.update_finalize_batch_data(
                &batch_id,
                tbd.state_chains,
                tbd.finalized_data_vec,
            )?;

        // If not batch then finalize transfer now
        } else {
            // Update DB and SMT with new transfer data
            self.transfer_finalize(&finalized_data)?;
        }

        info!(
            "TRANSFER: Receiver side complete. State Chain ID: {}",
            new_shared_key_id
        );
        debug!("TRANSFER: Receiver side complete. State Chain ID: {}. New Shared Key ID: {}. Finalized data: {:?}",state_chain_id,state_chain_id,finalized_data);

        Ok(TransferMsg5 {
            new_shared_key_id,
            s2_pub,
        })
    }

    /// Update DB and SMT after successful transfer.
    /// This function is called immediately in the regular transfer case or after confirmation of atomic
    /// transfers completion in the batch transfer case.
    fn transfer_finalize(&self, finalized_data: &TransferFinalizeData) -> Result<()> {
        let state_chain_id = finalized_data.state_chain_id;

        info!("TRANSFER_FINALIZE: State Chain ID: {}", state_chain_id);

        // Update state chain
        let mut state_chain: StateChain = self.database.get_statechain(state_chain_id)?;

        state_chain.add(finalized_data.state_chain_sig.to_owned())?;

        let new_user_id = finalized_data.new_shared_key_id;

        self.database.update_statechain_owner(
            &state_chain_id,
            state_chain.clone(),
            &new_user_id,
        )?;

        // Create new UserSession to allow new owner to generate shared wallet

        self.database.transfer_init_user_session(
            &new_user_id,
            &state_chain_id,
            finalized_data.to_owned(),
        )?;

        self.database
            .update_backup_tx(&state_chain_id, finalized_data.new_tx_backup.to_owned())?;

        info!(
            "TRANSFER: Finalized. New shared key ID: {}. State Chain ID: {}",
            finalized_data.new_shared_key_id, state_chain_id
        );

        // Update sparse merkle tree with new StateChain entry
        let (prev_root, new_root) = self.update_smt(
            &finalized_data
                .new_tx_backup
                .input
                .get(0)
                .unwrap()
                .previous_output
                .txid
                .to_string(),
            &state_chain
                .chain
                .last()
                .ok_or(SEError::Generic(String::from("StateChain empty")))?
                .data
                .clone(),
        )?;

        info!(
            "TRANSFER: Included in sparse merkle tree. State Chain ID: {}",
            state_chain_id
        );
        debug!(
            "TRANSFER: State Chain ID: {}. New root: {:?}. Previous root: {:?}.",
            state_chain_id, &new_root, &prev_root
        );

        // Remove TransferData for this transfer
        self.database.remove_transfer_data(&state_chain_id)?;

        Ok(())
    }
}

#[post("/transfer/sender", format = "json", data = "<transfer_msg1>")]
pub fn transfer_sender(
    sc_entity: State<SCE>,
    transfer_msg1: Json<TransferMsg1>,
) -> Result<Json<TransferMsg2>> {
    match sc_entity.transfer_sender(transfer_msg1.into_inner()) {
        Ok(res) => return Ok(Json(res)),
        Err(e) => return Err(e),
    }
}

#[post("/transfer/receiver", format = "json", data = "<transfer_msg4>")]
pub fn transfer_receiver(
    sc_entity: State<SCE>,
    transfer_msg4: Json<TransferMsg4>,
) -> Result<Json<TransferMsg5>> {
    match sc_entity.transfer_receiver(transfer_msg4.into_inner()) {
        Ok(res) => return Ok(Json(res)),
        Err(e) => return Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockDatabase;
    use crate::{
        error::DBErrorType,
        protocol::util::{
            mocks,
            tests::{test_sc_entity, BACKUP_TX_NOT_SIGNED, STATE_CHAIN},
        },
        structs::{ECDSAKeypair, StateChainOwner, TransferData, TransferFinalizeBatchData},
    };
    use chrono::{Duration, Utc};
    use mockall::predicate;
    use std::collections::HashMap;
    use std::str::FromStr;

    // Data from a run of transfer protocol.
    // static TRANSFER_MSG_1: &str = "{\"shared_key_id\":\"707ea4c9-5ddb-4f08-a240-2b4d80ae630d\",\"state_chain_sig\":{\"purpose\":\"TRANSFER\",\"data\":\"0213be735d05adea658d78df4719072a6debf152845044402c5fe09dd41879fa01\",\"sig\":\"3044022028d56cfdb4e02d46b2f8158b0414746ddf42ecaaaa995a3a02df8807c5062c0202207569dc0f49b64ae997b4c902539cddc1f4e4434d6b4b05af38af4b98232ebee8\"}}";
    static TRANSFER_MSG_2: &str = "{\"x1\":{\"secret_bytes\":[217,214,207,25,253,52,22,248,213,221,5,144,234,167,41,113,133,7,4,157,15,84,91,60,178,40,179,202,26,62,186,105]},\"proof_key\":\"04b42845b4e8477af2133ea5b5c15a4e8864e48d553c018a20ef6fa54526b8879c87306264ad3b4d1525ecf863734f6145a59cc940bb0a8813bedd9a8f6d816814\"}";

    // static TRANSFER_MSG_3: &str = "{\"shared_key_id\":\"707ea4c9-5ddb-4f08-a240-2b4d80ae630d\",\"t1\":\"34c9a329617b8dd3cdeb3d491fa09f023f84f28005bdf40f0682eb020969183b\",\"state_chain_sig\":{\"purpose\":\"TRANSFER\",\"data\":\"0213be735d05adea658d78df4719072a6debf152845044402c5fe09dd41879fa01\",\"sig\":\"3044022028d56cfdb4e02d46b2f8158b0414746ddf42ecaaaa995a3a02df8807c5062c0202207569dc0f49b64ae997b4c902539cddc1f4e4434d6b4b05af38af4b98232ebee8\"},\"state_chain_id\":\"9b0ba36b-406a-499c-8c83-696b77f003a9\",\"tx_backup_psm\":{\"shared_key_id\":\"707ea4c9-5ddb-4f08-a240-2b4d80ae630d\",\"protocol\":\"Transfer\",\"tx\":{\"version\":2,\"lock_time\":0,\"input\":[{\"previous_output\":\"53e1d67d837fdaddb016c5de85d8903bc033f7f2208d3ff40430fc42edeab4cb:0\",\"script_sig\":\"\",\"sequence\":4294967295,\"witness\":[[48,69,2,33,0,177,248,103,71,170,95,47,217,222,7,130,181,12,9,254,115,96,166,180,164,162,4,14,110,145,113,106,97,155,231,190,22,2,32,63,119,90,178,253,249,43,242,42,177,250,25,29,251,156,37,12,61,70,252,201,155,252,188,56,242,36,211,50,136,203,95,1],[2,108,195,112,80,86,19,121,166,106,134,63,140,162,115,194,178,158,147,92,173,6,188,127,94,107,131,160,62,11,191,241,230]]}],\"output\":[{\"value\":9000,\"script_pubkey\":\"0014a5c378a7de7311e6836253a28830b48cc6b9e252\"}]},\"input_addrs\":[\"026cc37050561379a66a863f8ca273c2b29e935cad06bc7f5e6b83a03e0bbff1e6\"],\"input_amounts\":[10000],\"proof_key\":\"0213be735d05adea658d78df4719072a6debf152845044402c5fe09dd41879fa01\"},\"rec_addr\":{\"tx_backup_addr\":\"bcrt1q5hph3f77wvg7dqmz2w3gsv953nrtncjjzyj3m9\",\"proof_key\":\"0213be735d05adea658d78df4719072a6debf152845044402c5fe09dd41879fa01\"}}";
    static TRANSFER_MSG_4: &str = "{\"shared_key_id\":\"707ea4c9-5ddb-4f08-a240-2b4d80ae630d\",\"state_chain_id\":\"9b0ba36b-406a-499c-8c83-696b77f003a9\",\"t2\":\"a1563a0006e1dac1cdb89d592327f7c5e292193365a0f15ebf805900261f9bb2\",\"state_chain_sig\":{\"purpose\":\"TRANSFER\",\"data\":\"0213be735d05adea658d78df4719072a6debf152845044402c5fe09dd41879fa01\",\"sig\":\"3044022028d56cfdb4e02d46b2f8158b0414746ddf42ecaaaa995a3a02df8807c5062c0202207569dc0f49b64ae997b4c902539cddc1f4e4434d6b4b05af38af4b98232ebee8\"},\"o2_pub\":{\"x\":\"e60171f570be0c6b673acbb5df775001b634e474e7ad329ab07b0fb50fead479\",\"y\":\"1ef781c8cde5310eb748a305dcab6b3ee302160d49d83b7ae8e7fde67979eb13\"},\"tx_backup\":{\"version\":2,\"lock_time\":0,\"input\":[{\"previous_output\":\"53e1d67d837fdaddb016c5de85d8903bc033f7f2208d3ff40430fc42edeab4cb:0\",\"script_sig\":\"\",\"sequence\":4294967295,\"witness\":[[48,69,2,33,0,177,248,103,71,170,95,47,217,222,7,130,181,12,9,254,115,96,166,180,164,162,4,14,110,145,113,106,97,155,231,190,22,2,32,63,119,90,178,253,249,43,242,42,177,250,25,29,251,156,37,12,61,70,252,201,155,252,188,56,242,36,211,50,136,203,95,1],[2,108,195,112,80,86,19,121,166,106,134,63,140,162,115,194,178,158,147,92,173,6,188,127,94,107,131,160,62,11,191,241,230]]}],\"output\":[{\"value\":9000,\"script_pubkey\":\"0014a5c378a7de7311e6836253a28830b48cc6b9e252\"}]},\"batch_data\":null}";
    static FINALIZED_DATA: &str = "{\"new_shared_key_id\":\"22f73737-efde-49a0-977a-ffaf8ba1e0f0\",\"state_chain_id\":\"9b0ba36b-406a-499c-8c83-696b77f003a9\",\"state_chain_sig\":{\"purpose\":\"TRANSFER\",\"data\":\"0213be735d05adea658d78df4719072a6debf152845044402c5fe09dd41879fa01\",\"sig\":\"3044022028d56cfdb4e02d46b2f8158b0414746ddf42ecaaaa995a3a02df8807c5062c0202207569dc0f49b64ae997b4c902539cddc1f4e4434d6b4b05af38af4b98232ebee8\"},\"s2\":\"28d85004c2a896df7f205882930ead6c7a95d84b3978174c51ebd06a4bd1589a\",\"new_tx_backup\":{\"version\":2,\"lock_time\":0,\"input\":[{\"previous_output\":\"53e1d67d837fdaddb016c5de85d8903bc033f7f2208d3ff40430fc42edeab4cb:0\",\"script_sig\":\"\",\"sequence\":4294967295,\"witness\":[[48,69,2,33,0,177,248,103,71,170,95,47,217,222,7,130,181,12,9,254,115,96,166,180,164,162,4,14,110,145,113,106,97,155,231,190,22,2,32,63,119,90,178,253,249,43,242,42,177,250,25,29,251,156,37,12,61,70,252,201,155,252,188,56,242,36,211,50,136,203,95,1],[2,108,195,112,80,86,19,121,166,106,134,63,140,162,115,194,178,158,147,92,173,6,188,127,94,107,131,160,62,11,191,241,230]]}],\"output\":[{\"value\":9000,\"script_pubkey\":\"0014a5c378a7de7311e6836253a28830b48cc6b9e252\"}]},\"batch_data\":null}";
    pub static PARTY_1_PRIVATE: &str = "{\"x1\":\"827089d12423e80ac4d6cd463d524326e3aa89c4623178df41a6581fec42fc4\",\"paillier_priv\":{\"p\":\"175105153600741631732008635643568979650827093652618445865555498830310239779193993937919065748609864882562533521325401979357004940357735331137242744377931301179917304999674039005453503946248939473532166164488354001195043141677905998318715771948374633284282386723061505364048790027483575020641965955188382828043\",\"q\":\"176107056094363704009530683741685388080833654947191096034654854567664678756371593133182239495448766868278040275902304993107585397542355074990977649321727244853545689372964609905231205840920297987033622047920439606987774726496544858149573923439784574804611753120265479364394401830948243108767573192431824915223\"},\"c_key_randomness\":\"c3d4d31f59de5dc74bd5f89a92d498197ea5fd93069556cde819db50b0fa9fc4649ee5f89404d943c2a227453defb2c58908869f13ec12897b150778c41dd037a6c88015e53be46beeed355ce2e41d8351005b06264f397cde4adde9d881e9abf3d4278a89b1d66beb335a4f81128e1e78e069a8ddfee1756585ff3aa80f714fe4f4ced8822b73a1d8c9c04375b76f055791a60b683443eb959ffb292aa152fd23561a69bfe20c1d711cc8be4a404591bf04cab07c472ca013e06b9b370cdb53a668af4f1646854a225a7cf07ea12e6c53f7d55014d445d2a1ed061e2320656a4afad19593f9de4fef4f0c73f018373a0eb61b7cd8c1d5efd1c485bd90b845bb\"}";
    pub static PARTY_2_PUBLIC: &str = "{\"x\":\"5220bc6ebcc83d0a1e4482ab1f2194cb69648100e8be78acde47ca56b996bd9e\",\"y\":\"8dfbb36ef76f2197598738329ffab7d3b3a06d80467db8e739c6b165abc20231\"}";

    #[test]
    fn test_transfer_sender() {
        let transfer_msg_4 = serde_json::from_str::<TransferMsg4>(&TRANSFER_MSG_4.to_string()).unwrap();
        let shared_key_id = transfer_msg_4.shared_key_id;
        let no_sc_shared_key_id = Uuid::from_str("deadb33f-1111-46f9-aaaa-0678c891b2d3").unwrap(); // random Uuid
        let state_chain_id = transfer_msg_4.state_chain_id;
        let state_chain_sig: StateChainSig =
            serde_json::from_str::<TransferMsg4>(&TRANSFER_MSG_4.to_string()).unwrap().state_chain_sig;
        let transfer_msg_1 = TransferMsg1 {shared_key_id,state_chain_sig};

        let mut db = MockDatabase::new();
        let (_privkey, pubkey) = shared_lib::util::keygen::generate_keypair();
        db.expect_get_proof_key().returning(move |_| Ok(pubkey.to_string()));
        db.expect_set_connection_from_config().returning(|_| Ok(()));
        db.expect_get_user_auth().returning(move |_| Ok(shared_key_id));
        db.expect_get_statechain_id()
            .with(predicate::eq(shared_key_id))
            .returning(move |_| Ok(state_chain_id));
        // userid does not own a state
        db.expect_get_statechain_id()
            .with(predicate::eq(no_sc_shared_key_id))
            .returning(move |_| {
                Err(SEError::DBError(
                    DBErrorType::NoDataForID,
                    no_sc_shared_key_id.to_string(),
                ))
            });
        db.expect_transfer_is_completed()
            .with(predicate::eq(state_chain_id))
            .returning(|_| false);
        db.expect_get_statechain_owner()    // sc locked
            .with(predicate::eq(state_chain_id))
            .times(1)
            .returning(move |_| {
                Ok(StateChainOwner {
                    locked_until: Utc::now().naive_utc() + Duration::seconds(5),
                    owner_id: shared_key_id,
                    chain: serde_json::from_str::<StateChain>(&STATE_CHAIN.to_string()).unwrap(),
                })
            });
        db.expect_get_statechain_owner()
            .with(predicate::eq(state_chain_id))
            .returning(move |_| {
                Ok(StateChainOwner {
                    locked_until: Utc::now().naive_utc(),
                    owner_id: shared_key_id,
                    chain: serde_json::from_str::<StateChain>(&STATE_CHAIN.to_string()).unwrap(),
                })
            });
        db.expect_create_transfer().returning(|_, _, _| Ok(()));

        let sc_entity = test_sc_entity(db);

        // user does not own State Chain
        let mut msg_1_wrong_shared_key_id = transfer_msg_1.clone();
        msg_1_wrong_shared_key_id.shared_key_id = no_sc_shared_key_id;
        match sc_entity.transfer_sender(msg_1_wrong_shared_key_id) {
            Ok(_) => assert!(false, "Expected failure."),
            Err(e) => assert!(e.to_string().contains("DB Error: No data for identifier.")),
        }

        // Sc locked
        match sc_entity.transfer_sender(transfer_msg_1.clone()) {
            Ok(_) => assert!(false, "Expected failure."),
            Err(e) => assert!(e.to_string().contains("SharedLibError Error: Error: State Chain locked for 1 minutes.")),
        }

        assert!(sc_entity
            .transfer_sender(transfer_msg_1)
            .is_ok());
    }

    #[test]
    fn test_transfer_receiver() {
        let transfer_msg_4 =
            serde_json::from_str::<TransferMsg4>(&TRANSFER_MSG_4.to_string()).unwrap();
        let shared_key_id = transfer_msg_4.shared_key_id;
        let state_chain_id = transfer_msg_4.state_chain_id;
        let s2 = serde_json::from_str::<TransferFinalizeData>(&FINALIZED_DATA.to_string())
            .unwrap()
            .s2;
        let msg2: TransferMsg2 = serde_json::from_str(&TRANSFER_MSG_2.to_string()).unwrap();
        let x1 = msg2.x1.get_fe().expect("failed to get fe");

        let mut db = MockDatabase::new();
        db.expect_set_connection_from_config().returning(|_| Ok(()));
        db.expect_get_user_auth()
            .returning(move |_| Ok(shared_key_id));
        db.expect_get_transfer_data()
            .with(predicate::eq(state_chain_id))
            .returning(move |_| {
                Ok(TransferData {
                    state_chain_id,
                    state_chain_sig: serde_json::from_str::<TransferMsg4>(
                        &TRANSFER_MSG_4.to_string(),
                    )
                    .unwrap()
                    .state_chain_sig,
                    x1,
                })
            });
        db.expect_get_ecdsa_keypair()
            .with(predicate::eq(shared_key_id))
            .returning(|_| {
                Ok(ECDSAKeypair {
                    party_1_private: serde_json::from_str(&PARTY_1_PRIVATE.to_string()).unwrap(),
                    party_2_public: serde_json::from_str(&PARTY_2_PUBLIC.to_string()).unwrap(),
                })
            });
        db.expect_get_statechain().returning(move |_| {
            Ok(serde_json::from_str::<StateChain>(&STATE_CHAIN.to_string()).unwrap())
        });
        db.expect_update_statechain_owner()
            .returning(|_, _, _| Ok(()));
        db.expect_transfer_init_user_session()
            .returning(|_, _, _| Ok(()));
        db.expect_update_backup_tx().returning(|_, _| Ok(()));
        db.expect_remove_transfer_data().returning(|_| Ok(()));
        db.expect_root_get_current_id().returning(|| Ok(1 as i64));
        db.expect_get_root().returning(|_| Ok(None));
        db.expect_root_update().returning(|_| Ok(1));
        db.expect_get_finalize_batch_data() // batch time up
            .times(1)
            .returning(move |_| {
                Ok(TransferFinalizeBatchData {
                    state_chains: HashMap::new(),
                    finalized_data_vec: vec![TransferFinalizeData {
                        new_shared_key_id: shared_key_id,
                        state_chain_id,
                        state_chain_sig: serde_json::from_str::<TransferMsg4>(
                            &TRANSFER_MSG_4.to_string(),
                        )
                        .unwrap()
                        .state_chain_sig,
                        s2: s2,
                        new_tx_backup: serde_json::from_str::<Transaction>(
                            &BACKUP_TX_NOT_SIGNED.to_string(),
                        )
                        .unwrap(),
                        batch_data: Some(BatchData {
                            id: shared_key_id,
                            commitment: String::default(),
                        }),
                    }],
                    start_time: Utc::now().naive_utc() - Duration::seconds(999999),
                })
            });
        db.expect_get_finalize_batch_data().returning(move |_| {
            Ok(TransferFinalizeBatchData {
                state_chains: HashMap::new(),
                finalized_data_vec: vec![TransferFinalizeData {
                    new_shared_key_id: shared_key_id,
                    state_chain_id,
                    state_chain_sig: serde_json::from_str::<TransferMsg4>(
                        &TRANSFER_MSG_4.to_string(),
                    )
                    .unwrap()
                    .state_chain_sig,
                    s2: s2,
                    new_tx_backup: serde_json::from_str::<Transaction>(
                        &BACKUP_TX_NOT_SIGNED.to_string(),
                    )
                    .unwrap(),
                    batch_data: Some(BatchData {
                        id: shared_key_id,
                        commitment: String::default(),
                    }),
                }],
                start_time: Utc::now().naive_utc(),
            })
        });
        db.expect_update_finalize_batch_data()
            .returning(|_, _, _| Ok(()));

        let sc_entity = test_sc_entity(db);
        let _m = mocks::ms::post_commitment().create();        //Mainstay post commitment mock

        // Input data to transfer_receiver
        let mut transfer_msg_4 =
            serde_json::from_str::<TransferMsg4>(&TRANSFER_MSG_4.to_string()).unwrap();

        // Incorrect x1, t1 or t2 => t2 is incorrect
        let mut msg_4_incorrect_t2 = transfer_msg_4.clone();

        //Generate an invalid x1 by adding x1 to itself
        let sk = x1.get_element();
        let x1_invalid = x1.add(&sk);
        msg_4_incorrect_t2.t2 = x1_invalid;

        match sc_entity.transfer_receiver(msg_4_incorrect_t2) {
            Ok(_) => assert!(false, "Expected failure."),
            Err(e) => assert!(e.to_string().contains("Transfer protocol error: P1 != P2")),
        }

        // StateChain incorreclty signed for
        let mut msg_4_incorrect_sc = transfer_msg_4.clone();
        msg_4_incorrect_sc.state_chain_sig.data =
            "deadb33f88579c6aafcfcc8ca91b0556a2044e6c61dfb7fca5f90c40ed119349ec".to_string();
        match sc_entity.transfer_receiver(msg_4_incorrect_sc) {
            Ok(_) => assert!(false, "Expected failure."),
            Err(e) => assert!(e
                .to_string()
                .contains("Error: State chain siganture provided does not match state chain at")),
        }
        // Expected successful run
        assert!(sc_entity.transfer_receiver(transfer_msg_4.clone()).is_ok());

        // Test transfer involved in batch
        transfer_msg_4.batch_data = Some(BatchData {
            id: shared_key_id,
            commitment: String::default(),
        });

        // Batch lifetime over
        match sc_entity.transfer_receiver(transfer_msg_4.clone()) {
            Ok(_) => assert!(false, "Expected failure."),
            Err(e) => assert!(e
                .to_string()
                .contains("Error: Transfer batch ended. Too late to complete transfer.")),
        }
        // Expected successful batch transfer run
        assert!(sc_entity.transfer_receiver(transfer_msg_4).is_ok());
    }
}
