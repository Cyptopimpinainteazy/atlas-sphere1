import fs from 'fs';
import path from 'path';
import solc from 'solc';
import { ethers } from 'ethers';

async function compile() {
  const source = fs.readFileSync(path.join(__dirname, '../contracts/HTLC.sol'), 'utf8');
  const input = {
    language: 'Solidity',
    sources: { 'HTLC.sol': { content: source } },
    settings: { outputSelection: { '*': { '*': ['abi', 'evm.bytecode'] } } }
  };
  const output = JSON.parse(solc.compile(JSON.stringify(input)));
  const contract = output.contracts['HTLC.sol']['HTLC'];
  return { abi: contract.abi, bytecode: contract.evm.bytecode.object };
}

async function main() {
  const providerUrl = process.env.EVM_RPC_URL;
  const pk = process.env.EVM_DEPLOYER_PRIVATE_KEY;
  if (!providerUrl || !pk) {
    console.error('EVM_RPC_URL and EVM_DEPLOYER_PRIVATE_KEY required');
    process.exit(1);
  }
  const { abi, bytecode } = await compile();
  const provider = new ethers.JsonRpcProvider(providerUrl);
  const wallet = new ethers.Wallet(pk, provider);
  console.log('Deploying HTLC with address', wallet.address);
  const factory = new ethers.ContractFactory(abi, bytecode, wallet);
  const contract = await factory.deploy({ gasLimit: 5_000_000 });
  await contract.waitForDeployment();
  console.log('Deployed HTLC at', contract.target);
  fs.writeFileSync(path.join(__dirname, '../deployed_htlc.json'), JSON.stringify({ address: contract.target, abi }, null, 2));
}

main().catch(err => { console.error(err); process.exit(1); });