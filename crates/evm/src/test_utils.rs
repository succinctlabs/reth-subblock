//! Helpers for testing.

use crate::{
    execute::{
        BasicBatchExecutor, BasicBlockExecutor, BatchExecutor, BlockExecutionInput,
        BlockExecutionOutput, BlockExecutionStrategy, BlockExecutorProvider, Executor,
    },
    system_calls::OnStateHook,
    ConfigureEvmEnv, NextBlockEnvAttributes,
};
use alloy_consensus::Header;
use alloy_eips::eip7685::Requests;
use alloy_primitives::BlockNumber;
use parking_lot::Mutex;
use reth_execution_errors::BlockExecutionError;
use reth_execution_types::ExecutionOutcome;
use reth_primitives::{BlockWithSenders, Receipt, Receipts, TransactionSigned};
use reth_prune_types::PruneModes;
use reth_storage_errors::provider::ProviderError;
use revm::State;
use revm_primitives::{
    db::Database, Address, BlockEnv, Bytes, CfgEnvWithHandlerCfg, Env, TxEnv, U256,
};
use std::{fmt::Display, sync::Arc};

/// A [`BlockExecutorProvider`] that returns mocked execution results.
#[derive(Clone, Debug, Default)]
pub struct MockExecutorProvider {
    exec_results: Arc<Mutex<Vec<ExecutionOutcome>>>,
}

impl MockExecutorProvider {
    /// Extend the mocked execution results
    pub fn extend(&self, results: impl IntoIterator<Item = impl Into<ExecutionOutcome>>) {
        self.exec_results.lock().extend(results.into_iter().map(Into::into));
    }
}

impl BlockExecutorProvider for MockExecutorProvider {
    type Executor<DB: Database<Error: Into<ProviderError> + Display>, EvmConfig: ConfigureEvmEnv> =
        Self;

    type BatchExecutor<
        DB: Database<Error: Into<ProviderError> + Display>,
        EvmConfig: ConfigureEvmEnv,
    > = Self;

    fn executor<DB, EvmConfig>(&self, _: DB) -> Self::Executor<DB, EvmConfig>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
        EvmConfig: ConfigureEvmEnv,
    {
        self.clone()
    }

    fn batch_executor<DB, EvmConfig>(&self, _: DB) -> Self::BatchExecutor<DB, EvmConfig>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
        EvmConfig: ConfigureEvmEnv,
    {
        self.clone()
    }
}

impl<DB, EvmConfig: ConfigureEvmEnv> Executor<DB, EvmConfig> for MockExecutorProvider {
    type Input<'a> = BlockExecutionInput<'a, BlockWithSenders>;
    type Output = BlockExecutionOutput<Receipt>;
    type Error = BlockExecutionError;

    fn execute(self, _: Self::Input<'_>) -> Result<Self::Output, Self::Error> {
        let ExecutionOutcome { bundle, receipts, requests, first_block: _ } =
            self.exec_results.lock().pop().unwrap();
        Ok(BlockExecutionOutput {
            state: bundle,
            receipts: receipts.into_iter().flatten().flatten().collect(),
            requests: requests.into_iter().fold(Requests::default(), |mut reqs, req| {
                reqs.extend(req);
                reqs
            }),
            gas_used: 0,
        })
    }

    fn execute_with_state_closure<F>(
        self,
        input: Self::Input<'_>,
        _: F,
    ) -> Result<Self::Output, Self::Error>
    where
        F: FnMut(&State<DB>),
    {
        <Self as Executor<DB, EvmConfig>>::execute(self, input)
    }

    fn execute_with_state_hook<F>(
        self,
        input: Self::Input<'_>,
        _: F,
    ) -> Result<Self::Output, Self::Error>
    where
        F: OnStateHook,
    {
        <Self as Executor<DB, EvmConfig>>::execute(self, input)
    }
}

impl<DB, EvmConfig: ConfigureEvmEnv> BatchExecutor<DB, EvmConfig> for MockExecutorProvider {
    type Input<'a> = BlockExecutionInput<'a, BlockWithSenders>;
    type Output = ExecutionOutcome;
    type Error = BlockExecutionError;

    fn execute_and_verify_one(&mut self, _: Self::Input<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn finalize(self) -> Self::Output {
        self.exec_results.lock().pop().unwrap()
    }

    fn set_tip(&mut self, _: BlockNumber) {}

    fn set_prune_modes(&mut self, _: PruneModes) {}

    fn size_hint(&self) -> Option<usize> {
        None
    }
}

impl<S, DB, EvmConfig> BasicBlockExecutor<S, DB, EvmConfig>
where
    S: BlockExecutionStrategy<DB, EvmConfig>,
    DB: Database,
    EvmConfig: ConfigureEvmEnv,
{
    /// Provides safe read access to the state
    pub fn with_state<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&State<DB>) -> R,
    {
        f(self.strategy.state_ref())
    }

    /// Provides safe write access to the state
    pub fn with_state_mut<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut State<DB>) -> R,
    {
        f(self.strategy.state_mut())
    }
}

impl<S, DB, EvmConfig> BasicBatchExecutor<S, DB, EvmConfig>
where
    S: BlockExecutionStrategy<DB, EvmConfig>,
    DB: Database,
    EvmConfig: ConfigureEvmEnv,
{
    /// Provides safe read access to the state
    pub fn with_state<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&State<DB>) -> R,
    {
        f(self.strategy.state_ref())
    }

    /// Provides safe write access to the state
    pub fn with_state_mut<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut State<DB>) -> R,
    {
        f(self.strategy.state_mut())
    }

    /// Accessor for batch executor receipts.
    pub const fn receipts(&self) -> &Receipts {
        self.batch_record.receipts()
    }
}

/// A noop `ConfigureEvmEnv`.
#[derive(Clone, Debug)]
pub struct TestEvmConfig {}

impl ConfigureEvmEnv for TestEvmConfig {
    type Header = Header;
    type Error = BlockExecutionError;

    fn fill_tx_env(&self, _tx_env: &mut TxEnv, _transaction: &TransactionSigned, _sender: Address) {
    }

    fn fill_tx_env_system_contract_call(
        &self,
        _env: &mut Env,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) {
    }

    fn fill_cfg_env(
        &self,
        _cfg_env: &mut CfgEnvWithHandlerCfg,
        _header: &Self::Header,
        _total_difficulty: U256,
    ) {
    }

    fn next_cfg_and_block_env(
        &self,
        _parent: &Self::Header,
        _attributes: NextBlockEnvAttributes,
    ) -> Result<(CfgEnvWithHandlerCfg, BlockEnv), Self::Error> {
        Err(BlockExecutionError::msg("test"))
    }
}
