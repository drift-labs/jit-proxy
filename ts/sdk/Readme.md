# Jit Proxy SDK

## Jit Proxy Client

```JitProxyClient``` will create and send transactions to the jit proxy program. Instantiate a jit proxy client with a drift client and a program Id. The current public key for the jit proxy program is

```J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP```. Example instantiation:

```
const jitProxyClient = new JitProxyClient({
			driftClient,
			programId: new PublicKey('J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP'),
		});
```

## Jitter

A jitter takes ```JitParams``` and uses them to determine when and how to use the ```JitProxyClient``` to send a transaction. For bots to make use of the jitter, they should create a jitter instance, and then set the ```JitParams``` according to their strategy for each market they are market making. ```JitParams``` are set on the Jitter object using ```updatePerpParams()``` and ```updateSpotParams()```. There are two kinds of Jitters that can be insantiated: ```JitterShotgun``` and ```JitterSniper```. The difference between the two is how orders are sent. 

For ```JitterShotgun```, orders are sent immediately when detecting a new eligible auction. The JitterShotgun will try up to 10 times to fill the order, retrying every time the it receives back an error due to the order not crossing the bid/ask in the ```JitParams``` or the oracle price being invalid. 

For ```JitterSniper```, orders are sent only when it detects that an order might cross the bid/ask of the ```JitParams```, waiting until the right slot, before sending up to 3 orders (retrying on errors). It will not send orders if the price of the order does not cross the bid/ask during the auction, unlike the JitterShotgun, which will immediately attempt.

## Jit Params

Type definition for the JitParmas is below:

```
export type JitParams = {
	bid: BN;
	ask: BN;
	minPosition: BN;
	maxPosition: BN;
	priceType: PriceType;
	subAccountId?: number;
};
```

PriceType options are ```ORACLE``` and ```LIMIT```. Limit price type is the BN representaiton of an absolute price; i.e., price type of LIMIT and a market of bid: 10, ask: 11 means your market is 10@11. Oracle price types are offsets relative to the oralce price for a given slot. They are always added to the oracle price, so if the oracle price is 10.5, to get a market of 10@11 when price type is oracle, bid and ask are -0.5 and 0.5 respectively (Remember that bid and ask are of BN type, this example is for illustration purposes only. Remember to use BN math operations).

## Example set up

Example set up for the JitterSniper (assuming parameters are already initialized/subscribed to). JitterShotgun is instantiated and initialized in a similar manner.

```
const jitProxyClient = new JitProxyClient({
			driftClient,
			programId: new PublicKey('J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP'),
		});

		const jitter = new JitterSniper({
			auctionSubscriber,
			driftClient,
			slotSubscriber,
			jitProxyClient,
		});
		await jitter.subscribe();
```
