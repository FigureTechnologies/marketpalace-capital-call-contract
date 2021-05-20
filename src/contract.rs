use cosmwasm_std::StdError;
use cosmwasm_std::{
    attr, entry_point, to_binary, BankMsg, Binary, Deps, DepsMut, Env, MessageInfo, Response,
    StdResult,
};
use provwasm_std::ProvenanceMsg;

use chrono::{DateTime, ParseResult, Utc};

use crate::error::ContractError;
use crate::msg::{HandleMsg, InstantiateMsg, QueryMsg};
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
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if info.funds.is_empty() {
        return Err(contract_error("no shares were committed"));
    }

    let deposit = info.funds.first().unwrap();

    match DateTime::parse_from_rfc3339(&msg.due_date_time) {
        ParseResult::Ok(due_date_time) => {
            if Utc::now() > due_date_time {
                return Err(contract_error("due date must be in future"));
            }
        }
        ParseResult::Err(_) => return Err(contract_error("unable to parse due date")),
    }

    let state = State {
        status: Status::PendingCapital,
        gp: info.sender,
        shares: deposit.clone(),
        distribution: msg.distribution,
        distribution_memo: msg.distribution_memo,
        lp_capital_source: msg.lp_capital_source,
        admin: msg.admin,
        capital: msg.capital,
        due_date_time: msg.due_date_time,
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
        HandleMsg::CommitCapital {} => try_commit_capital(deps, _env, info),
        HandleMsg::RecallCapital {} => try_recall_capital(deps, _env, info),
        HandleMsg::CallCapital {} => try_call_capital(deps, _env, info),
    }
}

fn is_past_due_date(state: &State) -> bool {
    Utc::now() > DateTime::parse_from_rfc3339(&state.due_date_time).unwrap()
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

    if is_past_due_date(&state) {
        return Err(contract_error("past due date"));
    }

    if info.sender != state.lp_capital_source {
        return Err(contract_error("wrong investor committing capital"));
    }

    if info.funds.is_empty() {
        return Err(contract_error("no capital was committed"));
    }

    let deposit = info.funds.first().unwrap();
    if deposit != &state.capital {
        return Err(contract_error("capital does not match required"));
    }

    config(deps.storage).update(|mut state| -> Result<_, ContractError> {
        state.status = Status::CapitalCommited;
        Ok(state)
    })?;

    Ok(Response {
        submessages: vec![],
        messages: vec![],
        attributes: vec![],
        data: Option::None,
    })
}

pub fn try_recall_capital(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response<ProvenanceMsg>, ContractError> {
    let state = config_read(deps.storage).load()?;

    if state.status != Status::CapitalCommited {
        return Err(contract_error("capital not committed"));
    }

    if is_past_due_date(&state) {
        return Err(contract_error("past due date"));
    }

    if info.sender != state.lp_capital_source {
        return Err(contract_error("wrong investor recalling capital"));
    }

    config(deps.storage).update(|mut state| -> Result<_, ContractError> {
        state.status = Status::PendingCapital;
        Ok(state)
    })?;

    Ok(Response {
        submessages: vec![],
        messages: vec![BankMsg::Send {
            to_address: state.lp_capital_source.to_string(),
            amount: vec![state.capital],
        }
        .into()],
        attributes: vec![],
        data: Option::None,
    })
}

pub fn try_call_capital(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response<ProvenanceMsg>, ContractError> {
    let state = config_read(deps.storage).load()?;

    if state.status != Status::CapitalCommited {
        return Err(contract_error("capital not committed"));
    }

    if info.sender != state.gp {
        return Err(contract_error("wrong gp calling capital"));
    }

    config(deps.storage).update(|mut state| -> Result<_, ContractError> {
        state.status = Status::CapitalCalled;
        Ok(state)
    })?;

    Ok(Response {
        submessages: vec![],
        messages: vec![
            BankMsg::Send {
                to_address: state.lp_capital_source.to_string(),
                amount: vec![state.shares],
            }
            .into(),
            BankMsg::Send {
                to_address: state.distribution.to_string(),
                amount: vec![state.capital],
            }
            .into(),
        ],
        attributes: vec![attr("memo", state.distribution_memo)],
        data: Option::None,
    })
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetStatus {} => to_binary(&query_status(deps)?),
    }
}

fn query_status(deps: Deps) -> StdResult<Status> {
    let state = config_read(deps.storage).load()?;
    Ok(state.status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coins, from_binary, Addr, Coin, Uint128};

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg {
            distribution: Addr::unchecked("tp1s2wz3pjz3emxuw3fq45fg7253kdxhf8fpjjtp4"),
            distribution_memo: String::from("54d402f2-2871-428f-b77b-029a98a67c7e"),
            lp_capital_source: Addr::unchecked("tp18lysxk7sueunnspju4dar34vlv98a7kyyfkqs7"),
            admin: Addr::unchecked("tp1apnhcu9x5cz2l8hhgnj0hg7ez53jah7hcan000"),
            capital: Coin {
                denom: String::from("cfigure"),
                amount: Uint128(1000000),
            },
            due_date_time: (Utc::now() + Duration::days(14)).to_rfc3339(),
        };
        let info = mock_info("creator", &coins(1000, "earth"));

        // we can just call .unwrap() to assert this was a success
        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetStatus {}).unwrap();
        let status: Status = from_binary(&res).unwrap();
        assert_eq!(Status::PendingCapital, status);
    }

    // #[test]
    // fn increment() {
    //     let mut deps = mock_dependencies(&coins(2, "token"));

    //     let msg = InstantiateMsg { count: 17 };
    //     let info = mock_info("creator", &coins(2, "token"));
    //     let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    //     // beneficiary can release it
    //     let info = mock_info("anyone", &coins(2, "token"));
    //     let msg = HandleMsg::Increment {};
    //     let _res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

    //     // should increase counter by 1
    //     let res = query(deps.as_ref(), mock_env(), QueryMsg::GetCount {}).unwrap();
    //     let value: CountResponse = from_binary(&res).unwrap();
    //     assert_eq!(18, value.count);
    // }

    // #[test]
    // fn reset() {
    //     let mut deps = mock_dependencies(&coins(2, "token"));

    //     let msg = InstantiateMsg { count: 17 };
    //     let info = mock_info("creator", &coins(2, "token"));
    //     let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    //     // beneficiary can release it
    //     let unauth_info = mock_info("anyone", &coins(2, "token"));
    //     let msg = HandleMsg::Reset { count: 5 };
    //     let res = execute(deps.as_mut(), mock_env(), unauth_info, msg);
    //     match res {
    //         Err(ContractError::Unauthorized {}) => {}
    //         _ => panic!("Must return unauthorized error"),
    //     }

    //     // only the original creator can reset the counter
    //     let auth_info = mock_info("creator", &coins(2, "token"));
    //     let msg = HandleMsg::Reset { count: 5 };
    //     let _res = execute(deps.as_mut(), mock_env(), auth_info, msg).unwrap();

    //     // should now be 5
    //     let res = query(deps.as_ref(), mock_env(), QueryMsg::GetCount {}).unwrap();
    //     let value: CountResponse = from_binary(&res).unwrap();
    //     assert_eq!(5, value.count);
    // }
}
