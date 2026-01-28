import HtlcRelayer from './handlers/htlc-relayer';
import fs from 'fs';
import path from 'path';

// Configuration
const EVM_RPC = process.env.EVM_RPC_URL || 'http://localhost:8545';
const EVM_DEPLOYER_PK = process.env.EVM_DEPLOYER_PRIVATE_KEY || '';
const EVM_HTLC_ARTIFACT = path.join(__dirname, '../../apps/blockchain-adapter/deployed_htlc.json');
const EVM_CONFIRMS = parseInt(process.env.EVM_CONFIRMS || '3', 10);
const X3_SIGNER_SURI = process.env.X3VM_SIGNER_SURI || '';
const X3_PALLET = process.env.X3HTLC_PALLET || 'x3Htlc';
const X3_METHOD = process.env.X3HTLC_METHOD || 'claim';
const FORWARD_FEE_WEI = process.env.FORWARD_FEE_WEI || '0'; // future use
const X3_FINALITY_DEPTH = parseInt(process.env.X3_FINALITY_DEPTH || '6', 10);
const SAFETY_DELTA_BLOCKS = parseInt(process.env.SAFETY_DELTA_BLOCKS || '20', 10); // blocks
const SECONDS_PER_BLOCK = parseInt(process.env.SECONDS_PER_BLOCK || '12', 10);
const SAFETY_DELTA_SECONDS = SAFETY_DELTA_BLOCKS * SECONDS_PER_BLOCK; // seconds to reserve before timelock expiry

if (!fs.existsSync(EVM_HTLC_ARTIFACT)) {
  console.error('[htlc-automation] deployed_htlc.json not found. Deploy HTLC first.');
  process.exit(1);
}

const evmDeployed = JSON.parse(fs.readFileSync(EVM_HTLC_ARTIFACT, 'utf8'));

async function sleep(ms:number){ return new Promise(r=>setTimeout(r, ms)); }

export class HtlcAutomator {
  relayer: HtlcRelayer;
  evmAbi: any;
  evmAddress: string;

  constructor() {
    const abiPath = path.join(__dirname, '../../apps/blockchain-adapter/deployed_htlc.json');
    this.evmAbi = evmDeployed.abi;
    this.evmAddress = evmDeployed.address;
    this.relayer = new HtlcRelayer(EVM_RPC, this.evmAbi, this.evmAddress);
  }

  async start() {
    await this.relayer.init();

    // watch EVM claims
    await this.relayer.watchEvmClaims(async (idHex: string, preimageHex: string, txHash: string, blockNumber: number) => {
      console.log('[htlc-automator] EVM claim heard', idHex, 'tx', txHash, 'block', blockNumber);
      // wait for confirmations
      const confirmed = await this.waitEvmConfirmations(txHash);
      if (!confirmed) {
        console.warn('[htlc-automator] EVM claim not yet confirmed, skipping');
        return;
      }
      // Submit to X3VM
      const payload = [idHex, Buffer.from(preimageHex.replace(/^0x/, ''), 'hex')];
      try {
        const x3BlockHash = await this.relayer.submitX3vmClaim(X3_SIGNER_SURI, X3_PALLET, X3_METHOD, payload);
        console.log('[htlc-automator] forwarded preimage to X3VM in block', x3BlockHash);
        // wait until X3VM finality depth is reached
        const finalized = await this.waitX3Finality(x3BlockHash);
        if (finalized) console.log('[htlc-automator] X3VM claim finalized');
        else console.warn('[htlc-automator] X3VM claim not finalized within timeout');
      } catch (err) {
        console.error('[htlc-automator] failed to forward to X3VM', err.message || err);
      }
    });

    // watch X3VM claims
    await this.relayer.watchX3vmClaims(async (htlcId: string, preimage: Uint8Array) => {
      console.log('[htlc-automator] X3VM claim heard', htlcId);
      // make a hex preimage
      const preHex = '0x' + Buffer.from(preimage).toString('hex');

      // sanity check: ensure EVM lock still has enough time left (safety delta)
      try {
        if (this.relayer.evmContract) {
          const lock = await (this.relayer.evmContract as any).getLock(htlcId);
          const timelock = lock[3] || lock.timelock; // timelock is the 4th return
          const now = Math.floor(Date.now() / 1000);
          const remaining = timelock - now;
          if (remaining < SAFETY_DELTA_SECONDS) {
            console.warn('[htlc-automator] EVM lock timelock too close or expired, remaining seconds', remaining, 'skipping claim');
            return;
          }
        }
      } catch (err) {
        console.warn('[htlc-automator] failed to verify EVM lock timelock, proceeding cautiously', err.message || err);
      }

      // forward to EVM
      try {
        const txhash = await this.relayer.submitEvmClaim(EVM_DEPLOYER_PK, htlcId, preHex);
        console.log('[htlc-automator] submitted claim to EVM tx', txhash);
        // wait confirmations
        const confirmed = await this.waitEvmConfirmations(txhash);
        if (!confirmed) console.warn('[htlc-automator] EVM claim not fully confirmed after submit');
      } catch (err) {
        console.error('[htlc-automator] failed to submit claim to EVM', err.message || err);
      }
    });

    // Reconciler loop: find stuck HTLCs and attempt recovery
    this.reconcilerLoop();

    console.log('[htlc-automator] started');
  }

  // Wait until an EVM tx has the required number of confirmations
  async waitEvmConfirmations(txHash: string, requiredConfirms = EVM_CONFIRMS, timeoutMs = 120000): Promise<boolean> {
    if (!txHash) return false;
    const start = Date.now();
    const provider = (this.relayer as any).evmProvider;
    while (Date.now() - start < timeoutMs) {
      try {
        const receipt = await provider.getTransactionReceipt(txHash);
        if (receipt && receipt.blockNumber) {
          const current = await provider.getBlockNumber();
          const confirms = current - receipt.blockNumber + 1; // include the block
          if (confirms >= requiredConfirms) return true;
          console.log('[htlc-automator] tx', txHash, 'confirms', confirms, 'waiting for', requiredConfirms);
        }
      } catch (err) {
        console.error('[htlc-automator] error while checking tx receipt', err.message || err);
      }
      await sleep(3000);
    }
    return false;
  }

  // Wait until a given X3 block hash is behind the finalized head by X3_FINALITY_DEPTH
  async waitX3Finality(blockHash: string, requiredDepth = X3_FINALITY_DEPTH, timeoutMs = 120000): Promise<boolean> {
    if (!blockHash) return false;
    const start = Date.now();
    const api = (this.relayer as any).api;
    while (Date.now() - start < timeoutMs) {
      try {
        const header = await api.rpc.chain.getHeader(blockHash);
        const finalizedHash = await api.rpc.chain.getFinalizedHead();
        const finalizedHeader = await api.rpc.chain.getHeader(finalizedHash);
        const blkNum = header.number.toNumber();
        const finalizedNum = finalizedHeader.number.toNumber();
        const depth = finalizedNum - blkNum;
        console.log('[htlc-automator] X3 block', blockHash, 'depth', depth, 'required', requiredDepth);
        if (depth >= requiredDepth) return true;
      } catch (err) {
        console.warn('[htlc-automator] waitX3Finality check failed', err.message || err);
      }
      await sleep(3000);
    }
    return false;
  }

  async reconcilerLoop() {
    while (true) {
      try {
        // TODO: query pending swaps, check timeouts, attempt refunds or alerts
        await sleep(30000);
      } catch (err) {
        console.error('[htlc-automator] reconciler error', err.message || err);
      }
    }
  }
}

if (require.main === module) {
  const automator = new HtlcAutomator();
  automator.start().catch(e=>{ console.error(e); process.exit(1); });
}

export default HtlcAutomator;