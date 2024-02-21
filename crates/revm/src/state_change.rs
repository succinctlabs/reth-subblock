use reth_interfaces::executor::{BlockExecutionError, BlockValidationError};
use reth_primitives::{
    constants::SYSTEM_ADDRESS, revm::env::fill_tx_env_with_beacon_root_contract_call, Address,
    ChainSpec, Header, Withdrawal, B256, U256,
};
use revm::{interpreter::Host, Database, DatabaseCommit, Evm};
use std::collections::HashMap;

/// Collect all balance changes at the end of the block.
///
/// Balance changes might include the block reward, uncle rewards, withdrawals, or irregular
/// state changes (DAO fork).
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn post_block_balance_increments(
    chain_spec: &ChainSpec,
    block_number: u64,
    block_difficulty: U256,
    beneficiary: Address,
    block_timestamp: u64,
    total_difficulty: U256,
    ommers: &[Header],
    withdrawals: Option<&[Withdrawal]>,
) -> HashMap<Address, u128> {
    let mut balance_increments = HashMap::new();

    // Add block rewards if they are enabled.
    if let Some(base_block_reward) =
        calc::base_block_reward(chain_spec, block_number, block_difficulty, total_difficulty)
    {
        // Ommer rewards
        for ommer in ommers {
            *balance_increments.entry(ommer.beneficiary).or_default() +=
                calc::ommer_reward(base_block_reward, block_number, ommer.number);
        }

        // Full block reward
        *balance_increments.entry(beneficiary).or_default() +=
            calc::block_reward(base_block_reward, ommers.len());
    }

    // process withdrawals
    insert_post_block_withdrawals_balance_increments(
        chain_spec,
        block_timestamp,
        withdrawals,
        &mut balance_increments,
    );

    balance_increments
}

/// Applies the pre-block call to the EIP-4788 beacon block root contract, using the given block,
/// [ChainSpec], EVM.
///
/// If cancun is not activated or the block is the genesis block, then this is a no-op, and no
/// state changes are made.
#[inline]
pub fn apply_beacon_root_contract_call<EXT, DB: Database + DatabaseCommit>(
    chain_spec: &ChainSpec,
    block_timestamp: u64,
    block_number: u64,
    parent_beacon_block_root: Option<B256>,
    evm: &mut Evm<'_, EXT, DB>,
) -> Result<(), BlockExecutionError>
where
    DB::Error: std::fmt::Display,
{
    if !chain_spec.is_cancun_active_at_timestamp(block_timestamp) {
        return Ok(())
    }

    let parent_beacon_block_root =
        parent_beacon_block_root.ok_or(BlockValidationError::MissingParentBeaconBlockRoot)?;

    // if the block number is zero (genesis block) then the parent beacon block root must
    // be 0x0 and no system transaction may occur as per EIP-4788
    if block_number == 0 {
        if parent_beacon_block_root != B256::ZERO {
            return Err(BlockValidationError::CancunGenesisParentBeaconBlockRootNotZero {
                parent_beacon_block_root,
            }
            .into())
        }
        return Ok(())
    }

    // get previous env
    let previous_env = Box::new(evm.env().clone());

    // modify env for pre block call
    fill_tx_env_with_beacon_root_contract_call(&mut evm.context.evm.env, parent_beacon_block_root);

    let mut state = match evm.transact() {
        Ok(res) => res.state,
        Err(e) => {
            evm.context.evm.env = previous_env;
            return Err(BlockValidationError::BeaconRootContractCall {
                parent_beacon_block_root: Box::new(parent_beacon_block_root),
                message: e.to_string(),
            }
            .into())
        }
    };

    state.remove(&SYSTEM_ADDRESS);
    state.remove(&evm.block().coinbase);

    evm.context.evm.db.commit(state);

    // re-set the previous env
    evm.context.evm.env = previous_env;

    Ok(())
}

/// Returns a map of addresses to their balance increments if the Shanghai hardfork is active at the
/// given timestamp.
///
/// Zero-valued withdrawals are filtered out.
#[inline]
pub fn post_block_withdrawals_balance_increments(
    chain_spec: &ChainSpec,
    block_timestamp: u64,
    withdrawals: &[Withdrawal],
) -> HashMap<Address, u128> {
    let mut balance_increments = HashMap::with_capacity(withdrawals.len());
    insert_post_block_withdrawals_balance_increments(
        chain_spec,
        block_timestamp,
        Some(withdrawals),
        &mut balance_increments,
    );
    balance_increments
}

/// Applies all withdrawal balance increments if shanghai is active at the given timestamp to the
/// given `balance_increments` map.
///
/// Zero-valued withdrawals are filtered out.
#[inline]
pub fn insert_post_block_withdrawals_balance_increments(
    chain_spec: &ChainSpec,
    block_timestamp: u64,
    withdrawals: Option<&[Withdrawal]>,
    balance_increments: &mut HashMap<Address, u128>,
) {
    // Process withdrawals
    if chain_spec.is_shanghai_active_at_timestamp(block_timestamp) {
        if let Some(withdrawals) = withdrawals {
            for withdrawal in withdrawals.iter() {
                if withdrawal.amount > 0 {
                    *balance_increments.entry(withdrawal.address).or_default() +=
                        withdrawal.amount_wei();
                }
            }
        }
    }
}

mod calc {

    use reth_primitives::{constants::ETH_TO_WEI, BlockNumber, Chain, ChainSpec, Hardfork, U256};
    /// Calculates the base block reward.
    ///
    /// The base block reward is defined as:
    ///
    /// - For Paris and later: `None`
    /// - For Petersburg and later: `Some(2 ETH)`
    /// - For Byzantium and later: `Some(3 ETH)`
    /// - Otherwise: `Some(5 ETH)`
    ///
    /// # Note
    ///
    /// This does not include the reward for including ommers. To calculate the full block reward,
    /// see [`block_reward`].
    ///
    /// # References
    ///
    /// - Definition: [Yellow Paper][yp] (page 15, 11.3)
    ///
    /// [yp]: https://ethereum.github.io/yellowpaper/paper.pdf
    pub fn base_block_reward(
        chain_spec: &ChainSpec,
        block_number: BlockNumber,
        block_difficulty: U256,
        total_difficulty: U256,
    ) -> Option<u128> {
        if chain_spec.chain == Chain::goerli() ||
            chain_spec.fork(Hardfork::Paris).active_at_ttd(total_difficulty, block_difficulty)
        {
            None
        } else if chain_spec.fork(Hardfork::Constantinople).active_at_block(block_number) {
            Some(ETH_TO_WEI * 2)
        } else if chain_spec.fork(Hardfork::Byzantium).active_at_block(block_number) {
            Some(ETH_TO_WEI * 3)
        } else {
            Some(ETH_TO_WEI * 5)
        }
    }

    /// Calculates the reward for a block, including the reward for ommer inclusion.
    ///
    /// The base reward should be calculated using [`base_block_reward`]. `ommers` represents the
    /// number of ommers included in the block.
    ///
    /// # Examples
    ///
    /// ```
    /// # use reth_consensus_common::calc::{base_block_reward, block_reward};
    /// # use reth_primitives::constants::ETH_TO_WEI;
    /// # use reth_primitives::{MAINNET, U256};
    /// #
    /// // This is block 126 on mainnet.
    /// let block_number = 126;
    /// let block_difficulty = U256::from(18_145_285_642usize);
    /// let total_difficulty = U256::from(2_235_668_675_900usize);
    /// let number_of_ommers = 1;
    ///
    /// let reward = base_block_reward(&MAINNET, block_number, block_difficulty, total_difficulty)
    ///     .map(|reward| block_reward(reward, 1));
    ///
    /// // The base block reward is 5 ETH, and the ommer inclusion reward is 1/32th of 5 ETH.
    /// assert_eq!(reward.unwrap(), ETH_TO_WEI * 5 + ((ETH_TO_WEI * 5) >> 5));
    /// ```
    ///
    /// # References
    ///
    /// - Definition: [Yellow Paper][yp] (page 15, 11.3)
    ///
    /// [yp]: https://ethereum.github.io/yellowpaper/paper.pdf
    pub fn block_reward(base_block_reward: u128, ommers: usize) -> u128 {
        base_block_reward + (base_block_reward >> 5) * ommers as u128
    }

    /// Calculate the reward for an ommer.
    ///
    /// # Application
    ///
    /// Rewards are accumulative, so they should be added to the beneficiary addresses in addition
    /// to any other rewards from the same block.
    ///
    /// From the yellow paper (page 15):
    ///
    /// > If there are collissions of the beneficiary addresses between ommers and the block (i.e.
    /// > two
    /// > ommers with the same beneficiary address or an ommer with the same beneficiary address as
    /// > the
    /// > present block), additions are applied cumulatively.
    ///
    /// # References
    ///
    /// - Implementation: [OpenEthereum][oe]
    /// - Definition: [Yellow Paper][yp] (page 15, 11.3)
    ///
    /// [oe]: https://github.com/openethereum/openethereum/blob/6c2d392d867b058ff867c4373e40850ca3f96969/crates/ethcore/src/ethereum/ethash.rs#L319-L333
    /// [yp]: https://ethereum.github.io/yellowpaper/paper.pdf
    pub fn ommer_reward(
        base_block_reward: u128,
        block_number: BlockNumber,
        ommer_block_number: BlockNumber,
    ) -> u128 {
        ((8 + ommer_block_number - block_number) as u128 * base_block_reward) >> 3
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use reth_primitives::{MAINNET, U256};

        #[test]
        fn calc_base_block_reward() {
            // ((block number, td), reward)
            let cases = [
                // Pre-byzantium
                ((0, U256::ZERO), Some(ETH_TO_WEI * 5)),
                // Byzantium
                ((4370000, U256::ZERO), Some(ETH_TO_WEI * 3)),
                // Petersburg
                ((7280000, U256::ZERO), Some(ETH_TO_WEI * 2)),
                // Merge
                ((10000000, U256::from(58_750_000_000_000_000_000_000_u128)), None),
            ];

            for ((block_number, td), expected_reward) in cases {
                assert_eq!(
                    base_block_reward(&MAINNET, block_number, U256::ZERO, td),
                    expected_reward
                );
            }
        }

        #[test]
        fn calc_full_block_reward() {
            let base_reward = ETH_TO_WEI;
            let one_thirty_twoth_reward = base_reward >> 5;

            // (num_ommers, reward)
            let cases = [
                (0, base_reward),
                (1, base_reward + one_thirty_twoth_reward),
                (2, base_reward + one_thirty_twoth_reward * 2),
            ];

            for (num_ommers, expected_reward) in cases {
                assert_eq!(block_reward(base_reward, num_ommers), expected_reward);
            }
        }
    }
}
