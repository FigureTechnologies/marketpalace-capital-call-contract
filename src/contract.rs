use cosmwasm_std::StdError;
use cosmwasm_std::{
    entry_point, to_binary, BankMsg, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use provwasm_std::ProvenanceMsg;

use crate::error::ContractError;
use crate::msg::{HandleMsg, InstantiateMsg, QueryMsg, Terms};
use crate::state::{config, config_read, State, Status};

fn contract_error(err: &str) -> ContractError {
    ContractError::Std(StdError::generic_err(err))
}

// Note, you can use StdResult in some functions where you do not
// make use of the custom errors
#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let state = State {
        status: Status::PendingCapital,
        raise: msg.raise,
        admin: msg.admin,
        subscription: msg.subscription,
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

    if info.sender != state.raise {
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

    if info.sender != state.raise {
        return Err(contract_error("only raise can close"));
    }

    let asset = match info.funds.first() {
        Some(asset) => asset,
        None => return Err(contract_error("must provide asset to close")),
    };

    if asset != &state.asset {
        return Err(contract_error(
            "must provide same asset denom and amount to close",
        ));
    }

    config(deps.storage).update(|mut state| -> Result<_, ContractError> {
        state.status = Status::CapitalCalled;
        Ok(state)
    })?;

    let send_asset = BankMsg::Send {
        to_address: state.subscription.to_string(),
        amount: vec![state.asset],
    }
    .into();

    let send_capital = BankMsg::Send {
        to_address: state.raise.to_string(),
        amount: vec![state.capital],
    }
    .into();

    Ok(Response {
        submessages: vec![],
        messages: vec![send_asset, send_capital],
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
    use cosmwasm_std::testing::{mock_env, mock_info};
    use cosmwasm_std::{coin, coins, from_binary, Addr, Coin, CosmosMsg};
    use provwasm_mocks::{mock_dependencies, must_read_binary_file};
    use provwasm_std::Marker;

    fn inst_msg() -> InstantiateMsg {
        InstantiateMsg {
            admin: Addr::unchecked("admin"),
            raise: Addr::unchecked("raise"),
            subscription: Addr::unchecked("sub_1"),
            capital: Coin::new(1000000, "stable_coin"),
            asset: Coin::new(10, "fund_coin"),
        }
    }

    fn is_send_msg(
        to: &'static str,
        amount: u128,
        denom: &'static str,
    ) -> Box<dyn Fn(&CosmosMsg<ProvenanceMsg>) -> bool> {
        Box::new(move |msg| match msg {
            CosmosMsg::Bank(bank) => match bank {
                BankMsg::Send {
                    to_address,
                    amount: coins,
                } => {
                    to_address == to && coins[0].amount.u128() == amount && coins[0].denom == denom
                }
                _ => false,
            },
            _ => false,
        })
    }

    #[test]
    fn initialization() {
        let mut deps = mock_dependencies(&vec![]);

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
        config(&mut deps.storage)
            .save(&State {
                status: Status::PendingCapital,
                raise: Addr::unchecked("raise"),
                admin: Addr::unchecked("admin"),
                subscription: Addr::unchecked("sub"),
                capital: coin(10_000, "stable_coin"),
                asset: coin(0, "fund_coin"),
            })
            .unwrap();

        // lp can commit capital
        let info = mock_info("lp", &coins(10_000, "stable_coin"));
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
        config(&mut deps.storage)
            .save(&State {
                status: Status::CapitalCommitted,
                raise: Addr::unchecked("raise"),
                admin: Addr::unchecked("admin"),
                subscription: Addr::unchecked("sub"),
                capital: coin(10_000, "stable_coin"),
                asset: coin(0, "fund_coin"),
            })
            .unwrap();

        // raise can cancel capital call
        let info = mock_info("raise", &[]);
        let msg = HandleMsg::Cancel {};
        let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        // should be in pending capital state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetStatus {}).unwrap();
        let status: Status = from_binary(&res).unwrap();
        assert_eq!(Status::Cancelled, status);

        // should send stable coin back to sub
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
        assert_eq!("sub", to_address);
        assert_eq!(10_000, u128::from(amount[0].amount));
        assert_eq!("stable_coin", amount[0].denom);
    }

    #[test]
    fn close() {
        // Create a mock querier with our expected marker.
        let bin = must_read_binary_file("testdata/marker.json");
        let expected_marker: Marker = from_binary(&bin).unwrap();
        let mut deps = mock_dependencies(&[]);
        deps.querier.with_markers(vec![expected_marker.clone()]);
        config(&mut deps.storage)
            .save(&State {
                status: Status::CapitalCommitted,
                raise: Addr::unchecked("raise"),
                admin: Addr::unchecked("admin"),
                subscription: Addr::unchecked("sub"),
                capital: coin(10_000, "stable_coin"),
                asset: coin(10_000, "fund_coin"),
            })
            .unwrap();

        // raise can close
        let info = mock_info("raise", &coins(10_000, "fund_coin"));
        let msg = HandleMsg::Close {};
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        assert_eq!(
            true,
            res.messages
                .iter()
                .any(is_send_msg("raise", 10_000, "stable_coin"))
        );
        assert_eq!(
            true,
            res.messages
                .iter()
                .any(is_send_msg("sub", 10_000, "fund_coin"))
        );

        // should be in capital called state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetStatus {}).unwrap();
        let status: Status = from_binary(&res).unwrap();
        assert_eq!(Status::CapitalCalled, status);
    }
}
