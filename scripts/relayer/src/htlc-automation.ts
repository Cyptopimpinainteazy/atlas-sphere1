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
    await this.relayer.watchEvmClaims(async (idHex: string, preimageHex: string) => {
      console.log('[htlc-automator] EVM claim heard', idHex);
      // wait for confirmations
      const confirmed = await this.waitEvmConfirmations(preimageHex); // dummy wrapper uses provider
      if (!confirmed) {
        console.warn('[htlc-automator] EVM claim not yet confirmed, skipping');
        return;
      }
      // Submit to X3VM
      const payload = [idHex, Buffer.from(preimageHex.replace(/^0x/, ''), 'hex')];
      try {
        const ok = await this.relayer.submitX3vmClaim(X3_SIGNER_SURI, X3_PALLET, X3_METHOD, payload);
        if (ok) console.log('[htlc-automator] forwarded preimage to X3VM');
      } catch (err) {
        console.error('[htlc-automator] failed to forward to X3VM', err.message || err);
      }
    });

    // watch X3VM claims
    await this.relayer.watchX3vmClaims(async (htlcId: string, preimage: Uint8Array) => {
      console.log('[htlc-automator] X3VM claim heard', htlcId);
      // make a hex preimage
      const preHex = '0x' + Buffer.from(preimage).toString('hex');
      // forward to EVM
      try {
        const txhash = await this.relayer.submitEvmClaim(EVM_DEPLOYER_PK, htlcId, preHex);
        console.log('[htlc-automator] submitted claim to EVM tx', txhash);
      } catch (err) {
        console.error('[htlc-automator] failed to submit claim to EVM', err.message || err);
      }
    });

    // Reconciler loop: find stuck HTLCs and attempt recovery
    this.reconcilerLoop();

    console.log('[htlc-automator] started');
  }

  // Placeholder - in real implementation check confirmations via provider
  async waitEvmConfirmations(preimageHex: string): Promise<boolean> {
    // Simple sleep for demo; in production check tx receipt confirmations
    await sleep(3000);
    return true;
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