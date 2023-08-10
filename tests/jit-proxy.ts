import * as anchor from '@coral-xyz/anchor';
import { Program } from '@coral-xyz/anchor';
import { JitProxy } from '../ts/sdk/src/types/jit_proxy';

describe('jit-proxy', () => {
	// Configure the client to use the local cluster.
	anchor.setProvider(anchor.AnchorProvider.env());

	const program = anchor.workspace.JitProxy as Program<JitProxy>;

	it('Is initialized!', async () => {
		// Add your test here.
		const tx = await program.methods.initialize().rpc();
		console.log('Your transaction signature', tx);
	});
});