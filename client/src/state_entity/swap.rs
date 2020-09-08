use super::super::Result;

use crate::error::{CError, WalletErrorType};
use crate::state_entity::{
    api::{get_smt_proof, get_smt_root, get_statechain},
    util::{cosign_tx_input, verify_statechain_smt},
};
use crate::wallet::{key_paths::funding_txid_to_int, wallet::Wallet};
use crate::{utilities::requests, ClientShim};
use shared_lib::{state_chain::StateChainSig, structs::*, 
    ecies::WalletDecryptable};

use bitcoin::{Address, PublicKey};
use curv::elliptic::curves::traits::{ECPoint, ECScalar};
use curv::{FE, GE};
use std::str::FromStr;
use uuid::Uuid;

fn poll_utxo(&self, state_chain_id: &Uuid) -> Result<Option<Uuid>>{
    requests::postb(
        &client_shim,
        &format!("/swap/poll/utxo"),
        state_chain_id,
    )
}

fn poll_swap(&self, swap_id: &Uuid) -> Result<Option<SwapStatus>>{
    requests::postb(
        &client_shim,
        &format!("/swap/poll/swap"),
        swap_id,
    )
}

fn get_swap_info(&self, swap_id: &Uuid) -> Result<Option<SwapInfo>>{
    requests::postb(
        &client_shim,
        &format!("/swap/info"),
        swap_id,
    )
}

fn register_utxo(&self, register_utxo_msg: &RegisterUtxo) -> Result<()>{
    requests::postb(
        &client_shim,
        &format!("/swap/register-utxo"),
        register_utxo_msg,
    )
}

fn swap_first_message(&self, swap_msg1: &SwapMsg1) -> Result<()>{
    requests::postb(
        &client_shim,
        &format!("/swap/first"),
        swap_msg_1,
    )
}

fn get_blinded_spend_token(&self, swap_id: &Uuid, statechain_id: &Uuid)
    -> Result<BlindedSpendToken>{
    let msg = RegisterUtxo {swap_id, statechain_id};
    requests::postb(
        &client_shim,
        &format!("/swap/blinded-spend-token"),
        &msg,
    )
}

fn swap_second_message(&self, swap_msg2: &SwapMsg2) -> Result<SCEAddress>{
    requests::postb(
        &client_shim,
        &format!("/swap/second"),
        swap_msg2,
    )
}

pub fn do_swap(&self, swap_size: &u32, wallet: &Wallet, state_chain_id: &Uuid) -> Result<()>{

    // 1) request to be included in swap
    // First sign state chain
    let state_chain_data: StateChainDataAPI = get_statechain(&wallet.client_shim, &state_chain_id)?;
    let state_chain = state_chain_data.chain;
    // Get proof key for signing
    let proof_key_derivation = wallet
        .se_proof_keys
        .get_key_derivation(&PublicKey::from_str(&state_chain.last().unwrap().data).unwrap());

    let proof_key_priv = &proof_key_derivation
    .ok_or(CError::WalletError(WalletErrorType::KeyNotFound))?
    .private_key
    .key;

    let signature = StateChainSig::new(
        proof_key_priv,
        &String::from("TRANSFER"),
        &receiver_addr.proof_key.clone().to_string(),
    )?;
    let register_msg = RegisterUtxo{state_chain_id, signature, swap_size};
    register_utxo(&register_msg)?;
    
    // 2) poll until included in swap
    let mut swap_id;
    loop {
        match poll_utxo(state_chain_id)?{
            Some(v) => {
                swap_id = v;
                break;
            },
            None => std::thread::sleep(std::time::Duration::from_secs(1))    
        };
    }
    loop {
        match poll_swap(&swap_id)?{
            Some(status) => match status {
                SwapStatus::Phase2 => break,
                _ => (),
            },
            None => ()
        };
    }
    //Now in phase 2
    let swap_info = get_swap_info(&swap_id)?.expect("expected swap info");
    //Assert still imn phase 2
    assert_eq!(swap_info.status, SwapStatus::Phase2, "expected to be in phase 2");

    //sign swap token
    let st_sig = swap_info.swap_token.sign(proof_key_priv).expect("failed to sign swap token");

    let bst = get_blinded_spend_token(&swap_id, state_chain_id).expect("expected blinded spend token");



    todo!();
}