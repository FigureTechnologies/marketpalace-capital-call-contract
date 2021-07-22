use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Coin};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub admin: Addr,
    pub raise: Addr,
    pub subscription: Addr,
    pub capital: Coin,
    pub asset: Coin,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    Cancel {},
    CommitCapital {},
    Close {},
}
#[derive(Deserialize, Serialize)]
pub struct Terms {
    pub subscription: Addr,
    pub raise: Addr,
    pub capital: Coin,
    pub asset: Coin,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    GetStatus {},
    GetTerms {},
}
