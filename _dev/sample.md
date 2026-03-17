# sample

typescript sample for reference

```ts
// file: src/services/polymarket_client.ts
// description: Polymarket UMA oracle client for reading on-chain price assertions
// reference: https://docs.polymarket.com, https://viem.sh/docs/actions/public/getLogs

import { createPublicClient, http, parseAbiItem, type PublicClient, type Log } from 'viem';
import { polygon } from 'viem/chains';
import { Logger } from '../utils/logger';
import { OracleSource, type OraclePrice } from '../types/oracle';

const UMA_OPTIMISTIC_ORACLE_V3_ADDRESS = '0x5953f2538F613E05bAED8A5AeFa8e6622467AD3D' as const;
const CTF_EXCHANGE_ADDRESS = '0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E' as const;
const CONDITIONAL_TOKENS_ADDRESS = '0x4D97DCd97eC945f40cF65F87097ACe5EA0476045' as const;

const UMA_ORACLE_ABI = [
  parseAbiItem('event AssertionMade(bytes32 indexed assertionId, bytes32 domainId, bytes claim, address indexed asserter, address callbackRecipient, address escalationManager, address caller, uint64 expirationTime, address currency, uint256 bond, bytes32 indexed identifier)'),
  parseAbiItem('event AssertionDisputed(bytes32 indexed assertionId, address indexed caller, address indexed disputer)'),
  parseAbiItem('event AssertionSettled(bytes32 indexed assertionId, address indexed bondRecipient, bool disputed, bool settlementResolution, address settleTo)'),
  {
    type: 'function',
    name: 'getAssertion',
    stateMutability: 'view',
    inputs: [
      {
        name: 'assertionId',
        type: 'bytes32'
      }
    ],
    outputs: [
      {
        name: 'settings',
        type: 'tuple',
        components: [
          { name: 'arbitrateViaEscalationManager', type: 'bool' },
          { name: 'discardOracle', type: 'bool' },
          { name: 'validateDisputers', type: 'bool' },
          { name: 'assertingCaller', type: 'address' },
          { name: 'escalationManager', type: 'address' }
        ]
      },
      { name: 'assertionTime', type: 'uint64' },
      { name: 'settled', type: 'bool' },
      { name: 'currency', type: 'address' },
      { name: 'expirationTime', type: 'uint64' },
      { name: 'settlementResolution', type: 'bool' },
      { name: 'domainId', type: 'bytes32' },
      { name: 'identifier', type: 'bytes32' },
      { name: 'bond', type: 'uint256' },
      { name: 'callbackRecipient', type: 'address' },
      { name: 'disputer', type: 'address' }
    ]
  } as const
] as const;

const CTF_EXCHANGE_ABI = [
  parseAbiItem('event OrderFilled(bytes32 indexed orderHash, address indexed maker, address indexed taker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled, uint256 fee)'),
  parseAbiItem('event OrdersMatched(bytes32 indexed takerOrderHash, address indexed takerOrderMaker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled)')
] as const;

interface PolymarketTrade {
  order_hash: string;
  maker: string;
  taker: string;
  maker_asset_id: bigint;
  taker_asset_id: bigint;
  maker_amount: bigint;
  taker_amount: bigint;
  price: number;
  timestamp: number;
  block_number: bigint;
}

interface UMAAssertion {
  assertion_id: string;
  claim: string;
  asserter: string;
  expiration_time: bigint;
  settled: boolean;
  settlement_resolution: boolean;
  timestamp: number;
  block_number: bigint;
}

export class PolymarketClient {
  private client: PublicClient;
  private rpc_url: string;

  constructor(rpc_url: string, hypersync_api_key?: string) {
    this.rpc_url = rpc_url;
    
    const transport_config: any = {
      url: rpc_url
    };

    if (hypersync_api_key && rpc_url.includes('hypersync')) {
      transport_config.fetchOptions = {
        headers: {
          'Authorization': `Bearer ${hypersync_api_key}`
        }
      };
    }

    this.client = createPublicClient({
      chain: polygon,
      transport: http(rpc_url, transport_config)
    });
  }

  async get_recent_trades(from_block: bigint, to_block?: bigint): Promise<PolymarketTrade[]> {
    try {
      const current_block = await this.client.getBlockNumber();
      const to = to_block || current_block;

      Logger.debug({
        at: 'PolymarketClient',
        message: 'Fetching order fills',
        from_block: from_block.toString(),
        to_block: to.toString()
      });

      const logs = await this.client.getLogs({
        address: CTF_EXCHANGE_ADDRESS,
        event: parseAbiItem('event OrderFilled(bytes32 indexed orderHash, address indexed maker, address indexed taker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled, uint256 fee)'),
        fromBlock: from_block,
        toBlock: to
      });

      const trades: PolymarketTrade[] = [];

      for (const log of logs) {
        if (!log.args) continue;

        const { orderHash, maker, taker, makerAssetId, takerAssetId, makerAmountFilled, takerAmountFilled } = log.args;

        if (!orderHash || !maker || !taker || makerAssetId === undefined || takerAssetId === undefined || 
            makerAmountFilled === undefined || takerAmountFilled === undefined) {
          continue;
        }

        const price = Number(takerAmountFilled) / Number(makerAmountFilled);

        const block = await this.client.getBlock({ blockNumber: log.blockNumber! });

        trades.push({
          order_hash: orderHash,
          maker,
          taker,
          maker_asset_id: makerAssetId,
          taker_asset_id: takerAssetId,
          maker_amount: makerAmountFilled,
          taker_amount: takerAmountFilled,
          price,
          timestamp: Number(block.timestamp),
          block_number: log.blockNumber!
        });
      }

      Logger.info({
        at: 'PolymarketClient',
        message: 'Trades fetched',
        count: trades.length
      });

      return trades;
    } catch (error) {
      Logger.error({
        at: 'PolymarketClient',
        message: 'Failed to fetch trades',
        error: error instanceof Error ? error.message : String(error)
      });
      return [];
    }
  }

  async get_uma_assertions(from_block: bigint, to_block?: bigint): Promise<UMAAssertion[]> {
    try {
      const current_block = await this.client.getBlockNumber();
      const to = to_block || current_block;

      Logger.debug({
        at: 'PolymarketClient',
        message: 'Fetching UMA assertions',
        from_block: from_block.toString(),
        to_block: to.toString()
      });

      const logs = await this.client.getLogs({
        address: UMA_OPTIMISTIC_ORACLE_V3_ADDRESS,
        event: parseAbiItem('event AssertionMade(bytes32 indexed assertionId, bytes32 domainId, bytes claim, address indexed asserter, address callbackRecipient, address escalationManager, address caller, uint64 expirationTime, address currency, uint256 bond, bytes32 indexed identifier)'),
        fromBlock: from_block,
        toBlock: to
      });

      const assertions: UMAAssertion[] = [];

      for (const log of logs) {
        if (!log.args) continue;

        const { assertionId, claim, asserter, expirationTime } = log.args;

        if (!assertionId || !claim || !asserter || expirationTime === undefined) {
          continue;
        }

        const block = await this.client.getBlock({ blockNumber: log.blockNumber! });

        const assertion_data = await this.client.readContract({
          address: UMA_OPTIMISTIC_ORACLE_V3_ADDRESS,
          abi: UMA_ORACLE_ABI,
          functionName: 'getAssertion',
          args: [assertionId]
        }) as readonly [
          { arbitrateViaEscalationManager: boolean; discardOracle: boolean; validateDisputers: boolean; assertingCaller: string; escalationManager: string },
          bigint,
          boolean,
          string,
          bigint,
          boolean,
          string,
          string,
          bigint,
          string,
          string
        ];

        assertions.push({
          assertion_id: assertionId,
          claim: Buffer.from(claim.slice(2), 'hex').toString('utf8'),
          asserter,
          expiration_time: expirationTime,
          settled: assertion_data[2],
          settlement_resolution: assertion_data[5],
          timestamp: Number(block.timestamp),
          block_number: log.blockNumber!
        });
      }

      Logger.info({
        at: 'PolymarketClient',
        message: 'UMA assertions fetched',
        count: assertions.length
      });

      return assertions;
    } catch (error) {
      Logger.error({
        at: 'PolymarketClient',
        message: 'Failed to fetch UMA assertions',
        error: error instanceof Error ? error.message : String(error)
      });
      return [];
    }
  }

  async get_market_price_from_trades(
    token_id: bigint,
    hours_back: number = 1
  ): Promise<OraclePrice | null> {
    try {
      const current_block = await this.client.getBlockNumber();
      const blocks_back = BigInt(Math.floor((hours_back * 3600) / 2));
      const from_block = current_block > blocks_back ? current_block - blocks_back : 0n;

      const trades = await this.get_recent_trades(from_block);

      const relevant_trades = trades.filter(
        t => t.maker_asset_id === token_id || t.taker_asset_id === token_id
      );

      if (relevant_trades.length === 0) {
        Logger.debug({
          at: 'PolymarketClient',
          message: 'No recent trades found',
          token_id: token_id.toString()
        });
        return null;
      }

      const prices = relevant_trades.map(t => t.price);
      const avg_price = prices.reduce((sum, p) => sum + p, 0) / prices.length;

      const latest_trade = relevant_trades[relevant_trades.length - 1];

      const oracle_price: OraclePrice = {
         source: OracleSource.POLYMARKET,
        asset: `POLYMARKET_TOKEN_${token_id.toString()}`,
        price: avg_price,
        timestamp: latest_trade.timestamp,
        decimals: 6,
        raw_value: BigInt(Math.floor(avg_price * 1e6)),
        block_number: latest_trade.block_number
      };

      Logger.debug({
        at: 'PolymarketClient',
        message: 'Market price calculated',
        token_id: token_id.toString(),
        price: avg_price,
        trades_count: relevant_trades.length
      });

      return oracle_price;
    } catch (error) {
      Logger.error({
        at: 'PolymarketClient',
        message: 'Failed to calculate market price',
        token_id: token_id.toString(),
        error: error instanceof Error ? error.message : String(error)
      });
      return null;
    }
  }

  async get_current_block(): Promise<bigint> {
    return await this.client.getBlockNumber();
  }

  async watch_trades(callback: (trade: PolymarketTrade) => void): Promise<void> {
    Logger.info({ at: 'PolymarketClient', message: 'Starting trade watcher' });

    const unwatch = this.client.watchEvent({
      address: CTF_EXCHANGE_ADDRESS,
      event: parseAbiItem('event OrderFilled(bytes32 indexed orderHash, address indexed maker, address indexed taker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled, uint256 fee)'),
      onLogs: async (logs) => {
        for (const log of logs) {
          if (!log.args) continue;

          const { orderHash, maker, taker, makerAssetId, takerAssetId, makerAmountFilled, takerAmountFilled } = log.args;

          if (!orderHash || !maker || !taker || makerAssetId === undefined || takerAssetId === undefined || 
              makerAmountFilled === undefined || takerAmountFilled === undefined) {
            continue;
          }

          const price = Number(takerAmountFilled) / Number(makerAmountFilled);
          const block = await this.client.getBlock({ blockNumber: log.blockNumber! });

          const trade: PolymarketTrade = {
            order_hash: orderHash,
            maker,
            taker,
            maker_asset_id: makerAssetId,
            taker_asset_id: takerAssetId,
            maker_amount: makerAmountFilled,
            taker_amount: takerAmountFilled,
            price,
            timestamp: Number(block.timestamp),
            block_number: log.blockNumber!
          };

          callback(trade);
        }
      }
    });
  }
}

```
