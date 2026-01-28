# HTLC Relayer

This relayer automates forwarding HTLC claims between EVM and X3VM.

Quick start (local dev):

- Start a local Substrate dev node (X3VM) and a local EVM network (Hardhat).
- Deploy the HTLC contract on EVM (see apps/blockchain-adapter/scripts/deploy_htlc.ts).
- Place the deployed artifact at `apps/blockchain-adapter/deployed_htlc.json`.

Environment variables (defaults shown):

- EVM_RPC_URL=http://localhost:8545
- EVM_DEPLOYER_PRIVATE_KEY=0x...
- EVM_CONFIRMS=3
- X3VM_WS_URL=ws://127.0.0.1:9944
- X3VM_SIGNER_SURI=//Alice
- X3HTLC_PALLET=x3Htlc
- X3HTLC_METHOD=claim
- X3_FINALITY_DEPTH=6
- SAFETY_DELTA_BLOCKS=20
- SECONDS_PER_BLOCK=12

Run the automator:

  yarn && npm run htlc:start

Notes:
- The automator now enforces EVM confirmation counts and waits for X3VM finality before considering a forwarded claim final.
- It also checks EVM lock timelocks to ensure there's enough time (safety delta) before attempting cross-chain claims.
