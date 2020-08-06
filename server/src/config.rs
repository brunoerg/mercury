//! # Config
//!
//! Config module handling config options from file and env

use super::Result;

use config_rs::{Config as ConfigRs, Environment, File};
use serde::{Deserialize, Serialize};
use shared_lib::mainstay::MainstayConfig;
use std::env;
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize, Clone)]
/// Storage specific config
pub struct StorageConfig {
    /// Storage write host
    pub db_host_w: String,
    /// Storage write port
    pub db_port_w: String,
    /// Storage write user
    pub db_user_w: String,
    /// Storage write pass
    pub db_pass_w: String,
    /// Storage write database
    pub db_database_w: String,
    /// Storage read host
    pub db_host_r: String,
    /// Storage read port
    pub db_port_r: String,
    /// Storage read user
    pub db_user_r: String,
    /// Storage read pass
    pub db_pass_r: String,
    /// Storage read database
    pub db_database_r: String,

}

impl Default for StorageConfig {
    fn default() -> StorageConfig {
        StorageConfig {
            db_host_w: String::from(""),
            db_port_w: String::from(""),
            db_user_w: String::from(""),
            db_pass_w: String::from(""),
            db_database_w: String::from(""),
            db_host_r: String::from(""),
            db_port_r: String::from(""),
            db_user_r: String::from(""),
            db_pass_r: String::from(""),
            db_database_r: String::from(""),
        }
    }
}

/// Config struct storing all StataChain Entity config
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Log file location. If not present print to stdout
    pub log_file: String,
    /// SMT DB location
    pub smt_db_loc: String,
    /// Electrum Server Address
    pub electrum_server: String,
    /// Bitcoin network name (testnet, regtest, mainnet)
    pub network: String,
    /// Block time of network
    pub block_time: u64,
    /// Testing mode
    pub testing_mode: bool,
    /// Receive address for fee payments
    pub fee_address: String,
    /// Despoit fee (satoshis)
    pub fee_deposit: u64,
    /// Withdraw fee (satoshis)
    pub fee_withdraw: u64,
    /// Time to allow batch transfer to take
    pub batch_lifetime: u64,
    /// Length of punishment for unresponsivve/misbehaving batch-transfer utxo
    pub punishment_duration: u64,
    /// Storage config
    pub storage: StorageConfig,
    /// Mainstay config
    pub mainstay: Option<MainstayConfig>,
    /// Database    
    pub database: PGDatabase
}


/// Config default variable definitons
pub static SMT_DB_LOC_DEFAULT: &str = "./db-smt";
pub static SMT_DB_LOC_TESTING: &str = "./db-smt-testing";

impl Default for Config {
    fn default() -> Config {
        Config {
            log_file: String::from(""),
            smt_db_loc: SMT_DB_LOC_DEFAULT.to_owned(),
            electrum_server: String::from("127.0.0.1:60401"),
            network: String::from("regtest"),
            block_time: 2,
            testing_mode: true,
            fee_address: String::from("bcrt1qjjwk2rk7nuxt6c79tsxthf5rpnky0sdhjr493x"),
            fee_deposit: 300,
            fee_withdraw: 300,
            batch_lifetime: 3600,     // 1 hour
            punishment_duration: 360, // 1 minute
            storage: StorageConfig::default(),
            mainstay: Some(MainstayConfig::default()),
        }
    }
}

impl Config {
    /// Load Config instance reading default values, overridden with Settings.toml,
    /// overriden with environment variables in form MERC_[setting_name]
    pub fn load() -> Result<Self> {
        let mut conf_rs = ConfigRs::new();
        let _ = conf_rs
            // First merge struct default config
            .merge(ConfigRs::try_from(&Config::default())?)?;
            // Override with settings in file Settings.toml if exists
            conf_rs.merge(File::with_name("Settings").required(false))?;
            // Override any config from env using MERC prefix
            conf_rs.merge(Environment::with_prefix("MERC"))?;

        // Override storage and mainstay config from env variables.
        // Currently doesn't seem to be supported by config_rs.
        // https://github.com/mehcode/config-rs/issues/104
        // A possible alternative would be using a "__" separator
        // e.g. Environment::with_prefix("CO").separator("__")) and
        // setting envs as below but is less readable and confusing
        // CO_CLIENTCHAIN__ASSET_HASH=73be005...
        // CO_CLIENTCHAIN__ASSET=CHALLENGE
        // CO_CLIENTCHAIN__HOST=127.0.0.1:5555
        // CO_CLIENTCHAIN__GENESIS_HASH=706f6...

        if let Ok(v) = env::var("MERC_DB_HOST_W") {
            let _ = conf_rs.set("storage.db_host_w", v)?;
        }
        if let Ok(v) = env::var("MERC_DB_PORT_W") {
            let _ = conf_rs.set("storage.db_port_w", v)?;
        }
        if let Ok(v) = env::var("MERC_DB_USER_W") {
            let _ = conf_rs.set("storage.db_user_w", v)?;
        }
        if let Ok(v) = env::var("MERC_DB_PASS_W") {
            let _ = conf_rs.set("storage.db_pass_w", v)?;
        }
        if let Ok(v) = env::var("MERC_DB_DATABASE_W") {
            let _ = conf_rs.set("storage.db_database_w", v)?;
        }

        if let Ok(v) = env::var("MERC_DB_HOST_R") {
            let _ = conf_rs.set("storage.db_host_r", v)?;
        }
        if let Ok(v) = env::var("MERC_DB_PORT_R") {
            let _ = conf_rs.set("storage.db_port_r", v)?;
        }
        if let Ok(v) = env::var("MERC_DB_USER_R") {
            let _ = conf_rs.set("storage.db_user_r", v)?;
        }
        if let Ok(v) = env::var("MERC_DB_PASS_R") {
            let _ = conf_rs.set("storage.db_pass_r", v)?;
        }
        if let Ok(v) = env::var("MERC_DB_DATABASE_R") {
            let _ = conf_rs.set("storage.db_database_r", v)?;
        }

        if let Ok(v) = env::var("MERC_MS_TEST_SLOT") {
            let _ = conf_rs.set("mainstay.postition", v)?;
        }
        if let Ok(v) = env::var("MERC_MS_TEST_TOKEN") {
            let _ = conf_rs.set("mainstay.token", v)?;
        }

        // Type checks
        let fee_address = conf_rs.get_str("fee_address")?;
        if let Err(e) = bitcoin::Address::from_str(&fee_address) {
            panic!("Invalid fee address: {}", e)
        };

        Ok(conf_rs.try_into()?)
    }
}
