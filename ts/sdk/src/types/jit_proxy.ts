export type JitProxy = {
	version: '0.1.0';
	name: 'jit_proxy';
	instructions: [
		{
			name: 'jit';
			accounts: [
				{
					name: 'state';
					isMut: false;
					isSigner: false;
				},
				{
					name: 'user';
					isMut: true;
					isSigner: false;
				},
				{
					name: 'userStats';
					isMut: true;
					isSigner: false;
				},
				{
					name: 'taker';
					isMut: true;
					isSigner: false;
				},
				{
					name: 'takerStats';
					isMut: true;
					isSigner: false;
				},
				{
					name: 'authority';
					isMut: false;
					isSigner: true;
				},
				{
					name: 'driftProgram';
					isMut: false;
					isSigner: false;
				}
			];
			args: [
				{
					name: 'params';
					type: {
						defined: 'JitParams';
					};
				}
			];
		}
	];
	errors: [
		{
			code: 6000;
			name: 'WorstPriceExceeded';
			msg: 'WorstPriceExceeded';
		},
		{
			code: 6001;
			name: 'TakerOrderNotFound';
			msg: 'TakerOrderNotFound';
		}
	];
};

export const IDL: JitProxy = {
	version: '0.1.0',
	name: 'jit_proxy',
	instructions: [
		{
			name: 'jit',
			accounts: [
				{
					name: 'state',
					isMut: false,
					isSigner: false,
				},
				{
					name: 'user',
					isMut: true,
					isSigner: false,
				},
				{
					name: 'userStats',
					isMut: true,
					isSigner: false,
				},
				{
					name: 'taker',
					isMut: true,
					isSigner: false,
				},
				{
					name: 'takerStats',
					isMut: true,
					isSigner: false,
				},
				{
					name: 'authority',
					isMut: false,
					isSigner: true,
				},
				{
					name: 'driftProgram',
					isMut: false,
					isSigner: false,
				},
			],
			args: [
				{
					name: 'params',
					type: {
						defined: 'JitParams',
					},
				},
			],
		},
	],
	errors: [
		{
			code: 6000,
			name: 'WorstPriceExceeded',
			msg: 'WorstPriceExceeded',
		},
		{
			code: 6001,
			name: 'TakerOrderNotFound',
			msg: 'TakerOrderNotFound',
		},
	],
};
