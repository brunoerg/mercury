//! API
//!
//! API calls availble for Client to State Entity

use super::super::Result;
use crate::wallet::wallet::Wallet;
use super::super::utilities::requests;
use shared_lib::structs::{StateChainData, SmtProofMsg};

use monotree::{Hash, Proof};


/// Get state chain by ID
pub fn get_statechain(wallet: &mut Wallet, state_chain_id: &String) -> Result<StateChainData> {
    requests::post(&wallet.client_shim,&format!("api/statechain/{}",state_chain_id))
}

/// Get state entity's sparse merkle tree root
pub fn get_smt_root(wallet: &mut Wallet) -> Result<Option<Hash>> {
    requests::post(&wallet.client_shim,&format!("/api/root"))
}

/// Get state chain inclusion proof
pub fn get_smt_proof(wallet: &mut Wallet, root: &Option<Hash>, funding_txid: &String) -> Result<Option<Proof>> {
    let smt_proof_msg = SmtProofMsg {
        root: root.clone(),
        funding_txid: funding_txid.clone()
    };
    requests::postb(&wallet.client_shim,&format!("api/proof"),smt_proof_msg)
}