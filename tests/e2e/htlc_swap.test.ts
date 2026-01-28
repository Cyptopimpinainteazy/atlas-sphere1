import { spawn, ChildProcess } from 'child_process';
import path from 'path';
import { ethers } from 'ethers';
import fs from 'fs';

const ADAPTER_DIR = path.join(__dirname, '..', '..', 'apps', 'blockchain-adapter');

describe('HTLC atomic swap e2e (dev-mode)', () => {
  let proc: ChildProcess | null = null;
  beforeAll(async () => {
    if (!process.env.EVM_DEPLOYER_PRIVATE_KEY || !process.env.X3VM_SIGNER_SURI) {
      console.warn('Skipping HTLC e2e tests: missing env keys');
      return;
    }
    proc = spawn('npm', ['run', 'start'], { cwd: ADAPTER_DIR, env: { ...process.env }, stdio: ['ignore', 'pipe', 'pipe'] });
    await new Promise((r) => setTimeout(r, 3000));
  }, 20000);

  afterAll(() => {
    if (proc) proc.kill();
  });

  test('deploy HTLC contract (EVM)', async () => {
    if (!process.env.EVM_DEPLOYER_PRIVATE_KEY || !process.env.EVM_RPC_URL) return;
    const res = await spawnSyncPromise('npm', ['run', 'deploy-contracts'], { cwd: ADAPTER_DIR, env: process.env });
    expect(res.code).toBe(0);
    const deployed = JSON.parse(fs.readFileSync(path.join(ADAPTER_DIR, 'scripts', 'deployed_htlc.json'), 'utf8'));
    expect(deployed.address).toBeTruthy();
  }, 60000);

  // More e2e tests will go here to create locks and trigger cross-chain claim
});

function spawnSyncPromise(cmd:string, args:string[], opts:any={}){
  return new Promise<{code:number, stdout:string, stderr:string}>((resolve)=>{
    const p = spawn(cmd, args, opts);
    let out=''; let err='';
    p.stdout?.on('data', d=>out+=d.toString());
    p.stderr?.on('data', d=>err+=d.toString());
    p.on('close', code=> resolve({code: code||0, stdout:out, stderr:err}));
  });
}
