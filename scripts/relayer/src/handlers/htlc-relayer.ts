import { ethers } from 'ethers';
import { ApiPromise, WsProvider } from '@polkadot/api';
import x3vmConfig from '../config/x3vm-config';
import axios from 'axios';

// Simple HTLC relayer: listens for claim events on EVM and X3VM and forwards preimages
export class HtlcRelayer {
  evmProvider: ethers.JsonRpcProvider;
  evmContract: ethers.Contract | null = null;
  x3Provider: WsProvider;
  api: ApiPromise | null = null;

  constructor(private evmRpcUrl: string, private evmAbi: any, private evmAddress: string) {
    this.evmProvider = new ethers.JsonRpcProvider(evmRpcUrl);
    this.x3Provider = new WsProvider(process.env.X3VM_WS_URL || x3vmConfig.wsUrl);
  }

  async init() {
    // init EVM contract
    this.evmContract = new ethers.Contract(this.evmAddress, this.evmAbi, this.evmProvider);
    // init X3VM API
    this.api = await ApiPromise.create({ provider: this.x3Provider });
  }

  // Listen for EVM Claimed events which include preimage
  async watchEvmClaims(callback: (id: string, preimage: string) => Promise<void>) {
    if (!this.evmContract) throw new Error('evmContract not initialized');
    this.evmContract.on('Claimed', async (id: string, claimer: string, preimage: string, event: any) => {
      try {
        console.log('[htlc-relayer] EVM Claimed', id, claimer);
        await callback(id, preimage);
      } catch (err) {
        console.error('error in EVM claim callback', err);
      }
    });
  }

  // Listen for X3VM HTLC Claim events (assume pallet name `x3Htlc` and event `Claimed`)
  async watchX3vmClaims(callback: (id: string, preimage: Uint8Array) => Promise<void>) {
    if (!this.api) throw new Error('X3 API not initialized');
    // This assumes pallet `x3_htlc` emits an event `Claimed(HtlcId, AccountId, preimage)`
    this.api.query.system.events((events: any) => {
      events.forEach(async (record: any) => {
        const { event } = record;
        const section = event.section;
        const method = event.method;
        if (section === 'x3Htlc' && method === 'Claimed') {
          const [htlcId, claimer, preimage] = event.data;
          console.log('[htlc-relayer] X3VM Claimed', htlcId.toString());
          await callback(htlcId.toString(), preimage.toU8a());
        }
      });
    });
  }

  // Submit claim to EVM (call claim on contract), preimage as hex
  async submitEvmClaim(signerPk: string, idHex: string, preimageHex: string) {
    const signer = new ethers.Wallet(signerPk, this.evmProvider);
    const contract = new ethers.Contract(this.evmAddress, this.evmAbi, signer);
    const tx = await contract.claim(idHex, preimageHex, { gasLimit: 500_000 });
    const receipt = await tx.wait();
    return receipt.transactionHash;
  }

  // Submit claim to X3VM via system.remark or via pallet call (prefers pallet)
  async submitX3vmClaim(signerSuri: string, pallet: string, method: string, payload: any) {
    if (!this.api) throw new Error('X3 API not initialized');
    const kr = require('@polkadot/keyring').default;
    const keyring = new kr({ type: 'sr25519' });
    const pair = keyring.addFromUri(signerSuri);
    // call pallet.method(payload)
    try {
      const call = (this.api.tx as any)[pallet][method](...payload);
      const unsub = await call.signAndSend(pair, (res: any) => {
        if (res.status.isInBlock) {
          console.log('[htlc-relayer] X3 claim included in block', res.status.asInBlock.toHex());
          unsub();
        }
      });
      return true;
    } catch (err) {
      console.error('[htlc-relayer] submitX3vmClaim failed', err.message || err);
      return false;
    }
  }
}

export default HtlcRelayer;