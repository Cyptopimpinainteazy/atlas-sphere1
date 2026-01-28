import { expect } from 'chai';
import sinon from 'sinon';
import HtlcRelayer from '../src/handlers/htlc-relayer';
import HtlcAutomator from '../src/htlc-automation';

// Unit test: mock HtlcRelayer behavior
describe('HTLC Automator (unit)', function () {
  it('forwards EVM claim to X3VM', async function () {
    // Create a fake relayer with methods stubbed
    const fakeRelayer: any = {
      init: sinon.fake.resolves(null),
      watchEvmClaims: sinon.fake(async (cb: any) => {
        // simulate an EVM claim callback invocation
        await cb('0xdeadbeef', '0x010203');
      }),
      watchX3vmClaims: sinon.fake.resolves(null),
      submitX3vmClaim: sinon.fake.resolves(true),
    };

    // Replace relayer in automator
    const automatorModule = require('../src/htlc-automation');
    const automator = new automatorModule.HtlcAutomator();
    automator.relayer = fakeRelayer;

    await automator.start();

    expect(fakeRelayer.submitX3vmClaim.calledOnce).to.be.true;
  });
});