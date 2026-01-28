import { expect } from 'chai';
import { ethers } from 'hardhat';

describe('HTLC contract', function () {
  it('lock -> claim flow', async function () {
    const [deployer, recipient] = await ethers.getSigners();
    const HTLC = await ethers.getContractFactory('HTLC');
    const htlc = await HTLC.deploy();
    await htlc.deployed();

    const id = ethers.utils.keccak256(ethers.utils.toUtf8Bytes('swap-1'));
    const preimage = ethers.utils.toUtf8Bytes('secret-xyz');
    const secretHash = ethers.utils.sha256(preimage);
    const amount = ethers.utils.parseEther('1.0');
    const timelock = Math.floor(Date.now() / 1000) + 60; // 60s

    // lock
    await expect(htlc.connect(deployer).lock(id, recipient.address, secretHash, timelock, { value: amount }))
      .to.emit(htlc, 'Locked')
      .withArgs(id, deployer.address, recipient.address, amount, timelock);

    // claim with wrong preimage should fail
    await expect(htlc.connect(recipient).claim(id, ethers.utils.toUtf8Bytes('wrong'))).to.be.revertedWith('HTLC: invalid preimage');

    // claim with correct preimage
    await expect(htlc.connect(recipient).claim(id, preimage))
      .to.emit(htlc, 'Claimed');

    // lock state should be claimed
    const info = await htlc.getLock(id);
    expect(info[5]).to.equal(true); // claimed flag
  });

  it('refund after timelock', async function () {
    const [deployer] = await ethers.getSigners();
    const HTLC = await ethers.getContractFactory('HTLC');
    const htlc = await HTLC.deploy();
    await htlc.deployed();

    const id = ethers.utils.keccak256(ethers.utils.toUtf8Bytes('swap-2'));
    const preimage = ethers.utils.toUtf8Bytes('secret-abc');
    const secretHash = ethers.utils.sha256(preimage);
    const amount = ethers.utils.parseEther('0.5');
    const timelock = Math.floor(Date.now() / 1000) + 1; // 1s

    await htlc.connect(deployer).lock(id, deployer.address, secretHash, timelock, { value: amount });

    // advance time by 2 seconds
    await ethers.provider.send('evm_increaseTime', [2]);
    await ethers.provider.send('evm_mine', []);

    await expect(htlc.connect(deployer).refund(id)).to.emit(htlc, 'Refunded');
  });
});