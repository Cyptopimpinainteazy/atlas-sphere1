# Frontier EVM Integration Plan

This document outlines how the Atlas Sphere runtime will integrate [Frontier](https://github.com/paritytech/frontier) to provide an Ethereum-compatible execution environment while preserving the **canonical ledger** enforced by the Atlas Kernel. The primary objective is to deliver a seamless dual-VM developer experience where EVM accounts and contracts share liquidity and state guarantees with the Solana VM (SVM) via Comit transactions.

---

## Implementation Goals

1. **Canonical Ledger First** – Eliminate duplicate balance accounting inside the EVM pallet by delegating all balance reads and writes to the Atlas Kernel.
2. **ATLAS-Denominated Gas** – Charge execution fees in the native ATLAS asset, enforcing fee withdrawals via Comit-validated balances.
3. **Unified Deployment Flow** – Ensure contract deployment, upgrades, and self-destruction follow the same Comit-based finality rules used by SVM programs.
4. **Ethereum RPC Compatibility** – Offer the standard Frontier JSON-RPC surface so wallets (e.g., MetaMask) and tooling continue to function with minimal configuration.
5. **Future Dual-VM Orchestration** – Lay the groundwork for Comit-driven cross-VM calls where an EVM transaction can atomically trigger SVM execution (and vice versa).

---

## Canonical Ledger Coupling

### Balance Reads
- Override `pallet_evm::Config::Currency` with an adapter that proxies to the Atlas Kernel `CanonicalLedger`.
- Map EVM `H160` accounts to substrate `AccountId` via `AccountMapping` inside the Atlas Kernel, ensuring every EVM account has a canonical balance entry.
- Provide read-through caching for frequently accessed balances to minimize on-chain storage decoding without reintroducing duplicate state.

### Balance Writes
- Hook into the EVM pallet’s `OnChargeTransaction` and `OnBalanceChange` pipelines to:
  - Call a new Atlas Kernel runtime API `apply_evm_delta(account, asset_id, delta)`.
  - Emit Comit events that can be used for cross-VM reconciliation or audits.
- Reject writes when the Atlas Kernel determines a Comit requirement is unmet (e.g., insufficient balance after considering pending Comit locks).

---

## Gas Accounting in ATLAS

- Configure the EVM pallet’s `FeeCalculator` to denominate gas in ATLAS.
- Implement a conversion layer for legacy gas price units (gwei) to ATLAS plancks to satisfy Ethereum tooling expectations.
- Deduct gas fees directly from canonical balances before transaction execution; if deduction fails, the transaction is rejected pre-dispatch.
- Credit block authors via the Atlas Kernel so fee receipts are tracked in the canonical ledger.

---

## Contract Deployment & Lifecycle

1. **Transaction Submission** – User submits an `eth_sendRawTransaction` RPC call; Frontier feeds it to the transaction pool.
2. **Pre-Execution Checks** – Runtime verifies:
   - Sufficient ATLAS balance in the Atlas Kernel.
   - Optional Comit references (for dual-VM flows) are valid and pending.
3. **Execution** – EVM pallet executes bytecode, producing state diffs.
4. **State Application** – Storage writes and balance changes call back into the Atlas Kernel, ensuring the canonical ledger reflects the deployed contract’s new code hash and any balance transfers.
5. **Post-Execution Events** – Emit Atlas Kernel events linking the `ComitId` (if provided) to the deployed contract address for cross-VM traceability.

Self-destruct flows follow the same pattern: canonical balances are reconciled through the Atlas Kernel, preventing orphaned funds or double-spends.

---

## Ethereum JSON-RPC Compatibility

- Expose the standard Frontier RPC endpoints (`eth_*`, `net_*`, `web3_*`) via the node service.
- Map chain metadata so clients recognize Atlas Sphere as an EVM-compatible network (unique `chainId`, `networkId`, genesis hash).
- Implement custom middleware for:
  - Translating ATLAS-denominated gas prices into gwei for UI friendliness.
  - Surfacing Comit state (e.g., pending dual-VM operations) via `eth_getTransactionReceipt` extensions.

### MetaMask Support

- Publish a chain configuration JSON (RPC URL, chain ID, native currency) for easy MetaMask addition.
- Provide browser-side scripts or wallet connectors that:
  - Fetch Atlas Kernel asset metadata for display.
  - Warn users when a transaction requires Comit finalization to settle (future enhancement).
- Ensure MetaMask extension is installed and active in the browser for seamless integration.

---

## Planned Milestones

| Milestone | Description | Status |
|-----------|-------------|--------|
| EVM pallet added to runtime | Wire frontend runtime dependencies and construct runtime entry | ⏳ Planned |
| Canonical ledger adapter | Implement `Currency` wrapper calling Atlas Kernel storage APIs | ⏳ Planned |
| ATLAS gas payments | Replace default fee handling with Atlas Kernel integration | ⏳ Planned |
| Frontier RPC wiring | Integrate RPC crates, expose standard Ethereum endpoints | ⏳ Planned |
| MetaMask onboarding kit | Publish chain settings and helper utilities | ⏳ Planned |
| Comit-aware EVM receipts | Extend receipts with Comit references for dual-VM atomicity | ⏳ Planned |

---

## Open Questions & Future Work

- **Cross-VM Atomicity** – Define how EVM contract calls can schedule SVM operations within the same Comit, including rollback semantics.
- **State Proofs** – Determine if EVM storage proofs should be committed into the Atlas Kernel for auditability.
- **Fee Markets** – Explore dynamic fee markets that consider both EVM congestion and Atlas Kernel Comit throughput.

---

### Next Steps

1. Finalize runtime configuration for `pallet-evm` and `pallet-ethereum` using the Atlas Kernel currency adapter.
2. Implement RPC layer glue in `node/service.rs` to serve Frontier endpoints.
3. Draft developer docs outlining how to deploy Solidity contracts and monitor Comit-linked events.
4. Prototype MetaMask connection flow against a local Atlas Sphere devnet.

By tying Frontier’s EVM execution directly to the Atlas Kernel, Atlas Sphere delivers Ethereum compatibility without compromising the unified asset model or Comit-based dual-VM guarantees.