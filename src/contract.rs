use cosmwasm_std::StdError;
use cosmwasm_std::{
    entry_point, to_binary, Addr, BankMsg, Binary, Deps, DepsMut, Env, MessageInfo, Response,
    StdResult,
};
use provwasm_std::{withdraw_coins, ProvenanceMsg};

use crate::error::ContractError;
use crate::msg::{HandleMsg, InstantiateMsg, QueryMsg, Terms};
use crate::state::{config, config_read, State, Status};
use crate::sub::{SubQueryMsg, SubTerms};

fn contract_error(err: &str) -> ContractError {
    ContractError::Std(StdError::generic_err(err))
}

// Note, you can use StdResult in some functions where you do not
// make use of the custom errors
#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let terms: SubTerms = deps
        .querier
        .query_wasm_smart(msg.subscription.clone(), &SubQueryMsg::GetTerms {})
        .expect("terms");

    let state = State {
        status: Status::PendingCapital,
        raise: terms.raise,
        subscription: msg.subscription,
        admin: info.sender,
        capital: msg.capital,
        asset: msg.asset,
    };
    config(deps.storage).save(&state)?;

    Ok(Response::default())
}

// And declare a custom Error variant for the ones where you will want to make use of it
#[entry_point]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: HandleMsg,
) -> Result<Response<ProvenanceMsg>, ContractError> {
    match msg {
        HandleMsg::Cancel {} => try_cancel(deps, _env, info),
        HandleMsg::CommitCapital {} => try_commit_capital(deps, _env, info),
        HandleMsg::Close {} => try_close_call(deps, _env, info),
    }
}

pub fn try_commit_capital(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response<ProvenanceMsg>, ContractError> {
    let state = config_read(deps.storage).load()?;

    if state.status != Status::PendingCapital {
        return Err(contract_error("contract no longer pending capital"));
    }

    if info.funds.is_empty() {
        return Err(contract_error("no capital was committed"));
    }

    let deposit = info.funds.first().unwrap();
    if deposit != &state.capital {
        return Err(contract_error("capital does not match required"));
    }

    config(deps.storage).update(|mut state| -> Result<_, ContractError> {
        state.status = Status::CapitalCommitted;
        Ok(state)
    })?;

    Ok(Response {
        submessages: vec![],
        messages: vec![],
        attributes: vec![],
        data: Option::None,
    })
}

pub fn try_cancel(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response<ProvenanceMsg>, ContractError> {
    let state = config_read(deps.storage).load()?;

    if state.status == Status::CapitalCalled {
        return Err(contract_error("capital already called"));
    } else if state.status == Status::Cancelled {
        return Err(contract_error("already cancelled"));
    }

    if info.sender != state.raise && info.sender != state.admin {
        return Err(contract_error("only raise can cancel"));
    }

    config(deps.storage).update(|mut state| -> Result<_, ContractError> {
        state.status = Status::Cancelled;
        Ok(state)
    })?;

    let send = BankMsg::Send {
        to_address: state.subscription.to_string(),
        amount: vec![state.capital],
    }
    .into();

    Ok(Response {
        submessages: vec![],
        messages: if state.status == Status::CapitalCommitted {
            vec![send]
        } else {
            vec![]
        },
        attributes: vec![],
        data: Option::None,
    })
}

pub fn try_close_call(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response<ProvenanceMsg>, ContractError> {
    let state = config_read(deps.storage).load()?;

    if state.status != Status::CapitalCommitted {
        return Err(contract_error("capital not committed"));
    }

    if info.sender != state.raise && info.sender != state.admin {
        return Err(contract_error("only raise can call capital"));
    }

    config(deps.storage).update(|mut state| -> Result<_, ContractError> {
        state.status = Status::CapitalCalled;
        Ok(state)
    })?;

    let withdraw = withdraw_coins(
        state.asset.denom.clone(),
        state.asset.amount.into(),
        state.asset.denom.clone(),
        state.subscription,
    )?;

    Ok(Response {
        submessages: vec![],
        messages: vec![
            withdraw,
            BankMsg::Send {
                to_address: state.raise.to_string(),
                amount: vec![state.capital],
            }
            .into(),
        ],
        attributes: vec![],
        data: Option::None,
    })
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let state = config_read(deps.storage).load()?;

    match msg {
        QueryMsg::GetStatus {} => to_binary(&state.status),
        QueryMsg::GetTerms {} => to_binary(&Terms {
            raise: state.raise,
            subscription: state.subscription,
            capital: state.capital,
            asset: state.asset,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::wasm_smart_mock_dependencies;
    use cosmwasm_std::testing::{mock_env, mock_info};
    use cosmwasm_std::{coins, from_binary, Addr, Coin, ContractResult, CosmosMsg, SystemError, SystemResult};
    use provwasm_mocks::{mock_dependencies, must_read_binary_file};
    use provwasm_std::{Marker, MarkerMsgParams, ProvenanceMsgParams};

    fn inst_msg() -> InstantiateMsg {
        InstantiateMsg {
            subscription: Addr::unchecked("sub_1"),
            capital: Coin::new(1000000, "stable_coin"),
            asset: Coin::new(10, "fund_coin"),
        }
    }

    #[test]
    fn initialization() {
        let mut deps =
            wasm_smart_mock_dependencies(|contract_addr, _msg| match &contract_addr[..] {
                "sub_1" => SystemResult::Ok(ContractResult::Ok(
                    to_binary(&SubTerms {
                        owner: Addr::unchecked("lp"),
                        raise: Addr::unchecked("raise"),
                        capital_denom: String::from("stable_coin"),
                        min_commitment: 10_000,
                        max_commitment: 50_000,
                    })
                    .unwrap(),
                )),
                _ => SystemResult::Err(SystemError::UnsupportedRequest {
                    kind: String::from("not mocked"),
                }),
            });

        // we can just call .unwrap() to assert this was a success
        let res = instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("creator", &[]),
            inst_msg(),
        )
        .unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetStatus {}).unwrap();
        let status: Status = from_binary(&res).unwrap();
        assert_eq!(Status::PendingCapital, status);
    }

    #[test]
    fn commit_capital() {
        let mut deps = mock_dependencies(&coins(2, "token"));

        let info = mock_info("creator", &[]);
        let _res = instantiate(deps.as_mut(), mock_env(), info, inst_msg()).unwrap();

        // lp can commit capital
        let info = mock_info(
            "tp18lysxk7sueunnspju4dar34vlv98a7kyyfkqs7",
            &coins(1000000, "cfigure"),
        );
        let msg = HandleMsg::CommitCapital {};
        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        // should be in capital commited state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetStatus {}).unwrap();
        let status: Status = from_binary(&res).unwrap();
        assert_eq!(Status::CapitalCommitted, status);
    }

    #[test]
    fn cancel() {
        let mut deps = mock_dependencies(&coins(2, "token"));

        let info = mock_info("creator", &[]);
        let _res = instantiate(deps.as_mut(), mock_env(), info, inst_msg()).unwrap();

        // lp can commit capital
        let info = mock_info(
            "tp18lysxk7sueunnspju4dar34vlv98a7kyyfkqs7",
            &coins(1000000, "cfigure"),
        );
        let msg = HandleMsg::CommitCapital {};
        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        // raise can cancel capital call
        let info = mock_info("creator", &[]);
        let msg = HandleMsg::Cancel {};
        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        // should be in pending capital state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetStatus {}).unwrap();
        let status: Status = from_binary(&res).unwrap();
        assert_eq!(Status::Cancelled, status);

        // should send stable coin back to lp
        let (to_address, amount) = _res
            .messages
            .iter()
            .find_map(|msg| match msg {
                CosmosMsg::Bank(bank) => match bank {
                    BankMsg::Send { to_address, amount } => Some((to_address, amount)),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        assert_eq!("tp1apnhcu9x5cz2l8hhgnj0hg7ez53jah7hcan000", to_address);
        assert_eq!(1000000, u128::from(amount[0].amount));
        assert_eq!("cfigure", amount[0].denom);
    }

    #[test]
    fn close() {
        // Create a mock querier with our expected marker.
        let bin = must_read_binary_file("testdata/marker.json");
        let expected_marker: Marker = from_binary(&bin).unwrap();
        let mut deps = mock_dependencies(&[]);
        deps.querier.with_markers(vec![expected_marker.clone()]);

        let info = mock_info("creator", &[]);
        let _res = instantiate(deps.as_mut(), mock_env(), info, inst_msg()).unwrap();

        // lp can commit capital
        let info = mock_info(
            "tp18lysxk7sueunnspju4dar34vlv98a7kyyfkqs7",
            &coins(1000000, "cfigure"),
        );
        let msg = HandleMsg::CommitCapital {};
        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        // raise can close
        let info = mock_info("creator", &vec![]);
        let msg = HandleMsg::Close {};
        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        let (withdraw_coin, withdraw_recipient) = _res
            .messages
            .iter()
            .find_map(|msg| match msg {
                CosmosMsg::Custom(custom) => match custom {
                    ProvenanceMsg {
                        route: _,
                        params,
                        version: _,
                    } => match params {
                        ProvenanceMsgParams::Marker(params) => match params {
                            MarkerMsgParams::WithdrawCoins {
                                marker_denom: _,
                                coin,
                                recipient,
                            } => Some((coin, recipient)),
                            _ => None,
                        },
                        _ => None,
                    },
                },
                _ => None,
            })
            .unwrap();
        assert_eq!(10, u128::from(withdraw_coin.amount));
        assert_eq!(
            "tp1apnhcu9x5cz2l8hhgnj0hg7ez53jah7hcan000",
            withdraw_recipient.to_string()
        );

        let (to_address, amount) = _res
            .messages
            .iter()
            .find_map(|msg| match msg {
                CosmosMsg::Bank(bank) => match bank {
                    BankMsg::Send { to_address, amount } => Some((to_address, amount)),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        assert_eq!("creator", to_address);
        assert_eq!(1000000, u128::from(amount[0].amount));
        assert_eq!("cfigure", amount[0].denom);

        // should be in capital called state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetStatus {}).unwrap();
        let status: Status = from_binary(&res).unwrap();
        assert_eq!(Status::CapitalCalled, status);
    }
}
