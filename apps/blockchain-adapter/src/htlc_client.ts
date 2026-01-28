import { ethers } from 'ethers';
import fs from 'fs';
import path from 'path';

const DEPLOYED_PATH = path.join(__dirname, '../deployed_htlc.json');
let deployed: any = null;
if (fs.existsSync(DEPLOYED_PATH)) deployed = JSON.parse(fs.readFileSync(DEPLOYED_PATH, 'utf8'));

export async function createLock(evmRpcUrl: string, pk: string, idHex: string, recipient: string, secretHashHex: string, timelock: number, amountWei: string) {
  if (!deployed) throw new Error('HTLC contract not deployed; run deploy script');
  const provider = new ethers.JsonRpcProvider(evmRpcUrl);
  const wallet = new ethers.Wallet(pk, provider);
  const contract = new ethers.Contract(deployed.address, deployed.abi, wallet);
  const tx = await contract.lock(idHex, recipient, secretHashHex, timelock, { value: amountWei });
  const receipt = await tx.wait();
  return receipt;
}

export async function claimLock(evmRpcUrl: string, pk: string, idHex: string, preimage: string) {
  if (!deployed) throw new Error('HTLC contract not deployed; run deploy script');
  const provider = new ethers.JsonRpcProvider(evmRpcUrl);
  const wallet = new ethers.Wallet(pk, provider);
  const contract = new ethers.Contract(deployed.address, deployed.abi, wallet);
  const tx = await contract.claim(idHex, preimage);
  const receipt = await tx.wait();
  return receipt;
}

export async function getLock(evmRpcUrl: string, idHex: string) {
  if (!deployed) throw new Error('HTLC contract not deployed; run deploy script');
  const provider = new ethers.JsonRpcProvider(evmRpcUrl);
  const contract = new ethers.Contract(deployed.address, deployed.abi, provider);
  const info = await contract.getLock(idHex);
  return info;
}
