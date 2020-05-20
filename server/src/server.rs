use super::routes::*;
use super::storage::db;
use super::Config;

use config;
use rocket;
use rocket::{Request, Rocket};
use rocksdb;

use std::collections::HashMap;


#[derive(Deserialize)]
pub struct AuthConfig {
    pub issuer: String,
    pub audience: String,
    pub region: String,
    pub pool_id: String,
}

impl AuthConfig {
    pub fn load(settings: HashMap<String, String>) -> AuthConfig {
        let issuer = settings.get("issuer").unwrap_or(&"".to_string()).to_owned();
        let audience = settings
            .get("audience")
            .unwrap_or(&"".to_string())
            .to_owned();
        let region = settings.get("region").unwrap_or(&"".to_string()).to_owned();
        let pool_id = settings
            .get("pool_id")
            .unwrap_or(&"".to_string())
            .to_owned();

        AuthConfig {
            issuer,
            audience,
            region,
            pool_id,
        }
    }
}

#[catch(500)]
fn internal_error() -> &'static str {
    "Internal server error"
}

#[catch(400)]
fn bad_request() -> &'static str {
    "Bad request"
}

#[catch(404)]
fn not_found(req: &Request) -> String {
    format!("Unknown route '{}'.", req.uri())
}

pub fn get_server() -> Rocket {
    let settings = get_settings_as_map();
    let db_config = Config {
        db: get_db(settings.clone())
    };

    let auth_config = AuthConfig::load(settings.clone());

    rocket::ignite()
        .register(catchers![internal_error, not_found, bad_request])
        .mount(
            "/",
            routes![
                ping::ping,
                ecdsa::first_message,
                ecdsa::second_message,
                ecdsa::third_message,
                ecdsa::fourth_message,
                ecdsa::chain_code_first_message,
                ecdsa::chain_code_second_message,
                ecdsa::sign_first,
                ecdsa::sign_second,
                ecdsa::recover,
                schnorr::keygen_first,
                schnorr::keygen_second,
                schnorr::keygen_third,
                schnorr::sign,
                state_entity::get_statechain,
                state_entity::get_smt_root,
                state_entity::get_smt_proof,
                state_entity::deposit_init,
                state_entity::prepare_sign_backup,
                state_entity::transfer_sender,
                state_entity::transfer_receiver
            ],
        )
        .manage(db_config)
        .manage(auth_config)
}

fn get_settings_as_map() -> HashMap<String, String> {
    let config_file = include_str!("../Settings.toml");
    let mut settings = config::Config::default();
    settings
        .merge(config::File::from_str(
            config_file,
            config::FileFormat::Toml,
        ))
        .unwrap()
        .merge(config::Environment::new())
        .unwrap();

    settings.try_into::<HashMap<String, String>>().unwrap()
}

fn get_db(_settings: HashMap<String, String>) -> db::DB {
    // let db_type_string = settings
    //     .get("db")
    //     .unwrap_or(&"local".to_string())
    //     .to_uppercase();
    // let db_type = db_type_string.as_str();
    // let env = settings
    //     .get("env")
    //     .unwrap_or(&"dev".to_string())
    //     .to_string();

    db::DB::Local(rocksdb::DB::open_default(db::DB_LOC).unwrap())
}
