// Copyright (c) Zefchain Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

#![cfg_attr(target_arch = "wasm32", no_main)]

mod state;

use async_trait::async_trait;
use linera_sdk::{base::WithContractAbi, Contract, ContractRuntime, SimpleStateStorage};
use thiserror::Error;

pub struct LlmContract {
    state: (),
}

linera_sdk::contract!(LlmContract);

impl WithContractAbi for LlmContract {
    type Abi = llm::LlmAbi;
}

#[async_trait]
impl Contract for LlmContract {
    type Error = ContractError;
    type Storage = SimpleStateStorage<Self>;
    type State = ();
    type Message = ();
    type InitializationArgument = ();
    type Parameters = ();

    async fn new(state: (), _runtime: ContractRuntime<Self>) -> Result<Self, Self::Error> {
        Ok(LlmContract { state })
    }

    fn state_mut(&mut self) -> &mut Self::State {
        &mut self.state
    }

    async fn initialize(&mut self, _value: ()) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn execute_operation(&mut self, _operation: ()) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn execute_message(&mut self, _message: ()) -> Result<(), Self::Error> {
        Err(ContractError::MessagesNotSupported)
    }
}

/// An error that can occur during the contract execution.
#[derive(Debug, Error)]
pub enum ContractError {
    /// Failed to deserialize BCS bytes
    #[error("Failed to deserialize BCS bytes")]
    BcsError(#[from] bcs::Error),

    /// Failed to deserialize JSON string
    #[error("Failed to deserialize JSON string")]
    JsonError(#[from] serde_json::Error),

    /// Llm application doesn't support any cross-chain messages.
    #[error("Llm application doesn't support any cross-chain messages")]
    MessagesNotSupported,
}
