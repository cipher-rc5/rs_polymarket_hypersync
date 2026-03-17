This file is a merged representation of a subset of the codebase, containing specifically included files and files not matching ignore patterns, combined into a single document by Repomix.

# File Summary

## Purpose
This file contains a packed representation of a subset of the repository's contents that is considered the most important context.
It is designed to be easily consumable by AI systems for analysis, code review,
or other automated processes.

## File Format
The content is organized as follows:
1. This summary section
2. Repository information
3. Directory structure
4. Repository files (if enabled)
5. Multiple file entries, each consisting of:
  a. A header with the file path (## File: path/to/file)
  b. The full contents of the file in a code block

## Usage Guidelines
- This file should be treated as read-only. Any changes should be made to the
  original repository files, not this packed version.
- When processing this file, use the file path to distinguish
  between different files in the repository.
- Be aware that this file may contain sensitive information. Handle it with
  the same level of security as you would the original repository.

## Notes
- Some files may have been excluded based on .gitignore rules and Repomix's configuration
- Binary files are not included in this packed representation. Please refer to the Repository Structure section for a complete list of file paths, including binary files
- Only files matching these patterns are included: src/**, .env.example, config.yaml, package.json, schema.graphql
- Files matching these patterns are excluded: src/**/__tests__/**, src/abis/**
- Files matching patterns in .gitignore are excluded
- Files matching default ignore patterns are excluded
- Files are sorted by Git change count (files with more changes are at the bottom)

# Directory Structure
```
src/
  effects/
    marketMetadata.ts
  handlers/
    ConditionalTokens.ts
    Exchange.ts
    FeeModule.ts
    FixedProductMarketMaker.ts
    FPMMFactory.ts
    NegRiskAdapter.ts
    UmaSportsOracle.ts
    Wallet.ts
  utils/
    constants.ts
    ctf.ts
    fpmm.ts
    negRisk.ts
    pnl.ts
    wallet.ts
.env.example
config.yaml
package.json
schema.graphql
```

# Files

## File: src/effects/marketMetadata.ts
```typescript
import { createEffect, S } from "envio";

export const getMarketMetadata = createEffect(
  {
    name: "getMarketMetadata",
    input: S.string, // tokenId
    output: S.union([
      S.schema({
        question: S.string,
        slug: S.string,
        outcomes: S.string,
        description: S.string,
        image: S.string,
        startDate: S.string,
        endDate: S.string,
      }),
      null,
    ]),
    cache: false,
    rateLimit: { calls: 280, per: 10_000 }, // 280 req / 10s — well under 300/10s limit
  },
  async ({ input: tokenId }) => {
    const res = await fetch(
      `https://gamma-api.polymarket.com/markets?clob_token_ids=${tokenId}`,
    );
    if (!res.ok) return null;

    const data = (await res.json()) as Array<{
      question?: string;
      slug?: string;
      outcomes?: string;
      description?: string;
      image?: string;
      startDate?: string;
      endDate?: string;
    }>;
    const market = data[0];
    if (!market) return null;

    return {
      question: market.question ?? "",
      slug: market.slug ?? "",
      outcomes: market.outcomes ?? "[]",
      description: market.description ?? "",
      image: market.image ?? "",
      startDate: market.startDate ?? "",
      endDate: market.endDate ?? "",
    };
  },
);
```

## File: src/handlers/ConditionalTokens.ts
```typescript
import { ConditionalTokens } from "generated";
import {
  USDC,
  NEG_RISK_ADAPTER,
  EXCHANGE,
  NEG_RISK_EXCHANGE,
  COLLATERAL_SCALE,
  FIFTY_CENTS,
} from "../utils/constants.js";
import { computePositionId } from "../utils/ctf.js";
import { getEventKey } from "../utils/negRisk.js";
import {
  updateUserPositionWithBuy,
  updateUserPositionWithSell,
  loadOrCreateUserPosition,
} from "../utils/pnl.js";

const USDC_LOWER = USDC.toLowerCase();
const NEG_RISK_ADAPTER_LOWER = NEG_RISK_ADAPTER.toLowerCase();
const EXCHANGE_LOWER = EXCHANGE.toLowerCase();
const NEG_RISK_EXCHANGE_LOWER = NEG_RISK_EXCHANGE.toLowerCase();
const NEG_RISK_WRAPPED = "0x3A3BD7bb9528E159577F7C2e685CC81A765002E2" as `0x${string}`;

// Addresses to skip for activity tracking (handled elsewhere)
const SKIP_ACTIVITY = new Set([
  NEG_RISK_ADAPTER_LOWER,
  EXCHANGE_LOWER,
  NEG_RISK_EXCHANGE_LOWER,
]);

// Skip PnL for these (handled in their own handlers)
const SKIP_PNL = new Set([
  NEG_RISK_ADAPTER_LOWER,
  EXCHANGE_LOWER,
  NEG_RISK_EXCHANGE_LOWER,
]);

// ============================================================
// Helper: get or create OI entities
// ============================================================

async function getOrCreateMarketOI(
  context: any,
  conditionId: string,
): Promise<{ id: string; amount: bigint }> {
  const existing = await context.MarketOpenInterest.get(conditionId);
  if (existing) return existing;
  return { id: conditionId, amount: 0n };
}

async function getOrCreateGlobalOI(
  context: any,
): Promise<{ id: string; amount: bigint }> {
  const existing = await context.GlobalOpenInterest.get("");
  if (existing) return existing;
  return { id: "", amount: 0n };
}

async function updateOpenInterest(
  context: any,
  conditionId: string,
  amount: bigint,
): Promise<void> {
  const marketOI = await getOrCreateMarketOI(context, conditionId);
  context.MarketOpenInterest.set({
    ...marketOI,
    amount: marketOI.amount + amount,
  });

  const globalOI = await getOrCreateGlobalOI(context);
  context.GlobalOpenInterest.set({
    ...globalOI,
    amount: globalOI.amount + amount,
  });
}

// ============================================================
// Helper: compute position IDs for a condition
// ============================================================

function getPositionIds(
  conditionId: `0x${string}`,
  negRisk: boolean,
): [bigint, bigint] {
  const collateral = negRisk ? NEG_RISK_WRAPPED : (USDC as `0x${string}`);
  return [
    computePositionId(collateral, conditionId, 0),
    computePositionId(collateral, conditionId, 1),
  ];
}

// ============================================================
// ConditionPreparation — create Condition + Position entities
// ============================================================

ConditionalTokens.ConditionPreparation.handler(async ({ event, context }) => {
  // Only handle binary conditions (2 outcomes)
  if (event.params.outcomeSlotCount !== 2n) return;

  const conditionId = event.params.conditionId;
  const negRisk =
    event.params.oracle.toLowerCase() === NEG_RISK_ADAPTER_LOWER;

  // Compute position IDs for PnL tracking
  const [posId0, posId1] = getPositionIds(conditionId as `0x${string}`, negRisk);

  // Create Condition entity with position IDs (OI + PnL)
  const existing = await context.Condition.get(conditionId);
  if (!existing) {
    context.Condition.set({
      id: conditionId,
      positionIds: [posId0, posId1],
      payoutNumerators: [] as bigint[],
      payoutDenominator: 0n,
    });
  }

  // Create Position entities (Activity)
  for (let outcomeIndex = 0; outcomeIndex < 2; outcomeIndex++) {
    const positionId = outcomeIndex === 0 ? posId0 : posId1;
    const posIdStr = positionId.toString();

    const existingPos = await context.Position.get(posIdStr);
    if (!existingPos) {
      context.Position.set({
        id: posIdStr,
        condition: conditionId,
        outcomeIndex: BigInt(outcomeIndex),
      });
    }
  }
});

// ============================================================
// ConditionResolution — store payout numerators/denominator (PnL)
// ============================================================

ConditionalTokens.ConditionResolution.handler(async ({ event, context }) => {
  const conditionId = event.params.conditionId;
  const condition = await context.Condition.get(conditionId);
  if (!condition) return;

  const payoutNumerators = event.params.payoutNumerators.map((v: bigint) => v);
  const payoutDenominator = payoutNumerators.reduce(
    (sum: bigint, v: bigint) => sum + v,
    0n,
  );

  context.Condition.set({
    ...condition,
    payoutNumerators,
    payoutDenominator,
  });
});

// ============================================================
// PositionSplit — Activity + OI + PnL
// ============================================================

ConditionalTokens.PositionSplit.handler(async ({ event, context }) => {
  const conditionId = event.params.conditionId;
  const stakeholder = event.params.stakeholder;
  const stakeholderLower = stakeholder.toLowerCase();
  const collateralToken = event.params.collateralToken;

  const condition = await context.Condition.get(conditionId);
  if (!condition) return;

  // Activity: Create Split (skip FPMMs, NegRiskAdapter, Exchange)
  if (!SKIP_ACTIVITY.has(stakeholderLower)) {
    const isFPMM = await context.FixedProductMarketMaker.get(stakeholder);
    if (!isFPMM) {
      context.Split.set({
        id: getEventKey(event.chainId, event.block.number, event.logIndex),
        timestamp: BigInt(event.block.timestamp),
        stakeholder,
        condition: conditionId,
        amount: event.params.amount,
      });
    }
  }

  // OI: Only track USDC splits
  if (collateralToken.toLowerCase() === USDC_LOWER) {
    await updateOpenInterest(context, conditionId, event.params.amount);
  }

  // PnL: Split = buying both outcomes at 50 cents each (skip NRA/Exchange)
  if (!SKIP_PNL.has(stakeholderLower)) {
    const positionIds = condition.positionIds;
    for (let i = 0; i < 2; i++) {
      await updateUserPositionWithBuy(
        context,
        stakeholder,
        positionIds[i]!,
        FIFTY_CENTS,
        event.params.amount,
      );
    }
  }
});

// ============================================================
// PositionsMerge — Activity + OI + PnL
// ============================================================

ConditionalTokens.PositionsMerge.handler(async ({ event, context }) => {
  const conditionId = event.params.conditionId;
  const stakeholder = event.params.stakeholder;
  const stakeholderLower = stakeholder.toLowerCase();
  const collateralToken = event.params.collateralToken;

  const condition = await context.Condition.get(conditionId);
  if (!condition) return;

  // Activity: Create Merge (skip FPMMs, NegRiskAdapter, Exchange)
  if (!SKIP_ACTIVITY.has(stakeholderLower)) {
    const isFPMM = await context.FixedProductMarketMaker.get(stakeholder);
    if (!isFPMM) {
      context.Merge.set({
        id: getEventKey(event.chainId, event.block.number, event.logIndex),
        timestamp: BigInt(event.block.timestamp),
        stakeholder,
        condition: conditionId,
        amount: event.params.amount,
      });
    }
  }

  // OI: Only track USDC merges
  if (collateralToken.toLowerCase() === USDC_LOWER) {
    await updateOpenInterest(context, conditionId, -event.params.amount);
  }

  // PnL: Merge = selling both outcomes at 50 cents each (skip NRA/Exchange)
  if (!SKIP_PNL.has(stakeholderLower)) {
    const positionIds = condition.positionIds;
    for (let i = 0; i < 2; i++) {
      await updateUserPositionWithSell(
        context,
        stakeholder,
        positionIds[i]!,
        FIFTY_CENTS,
        event.params.amount,
      );
    }
  }
});

// ============================================================
// PayoutRedemption — Activity + OI + PnL
// ============================================================

ConditionalTokens.PayoutRedemption.handler(async ({ event, context }) => {
  const conditionId = event.params.conditionId;
  const redeemer = event.params.redeemer;
  const collateralToken = event.params.collateralToken;

  const condition = await context.Condition.get(conditionId);
  if (!condition) return;

  // Activity: Create Redemption (skip NegRiskAdapter)
  if (redeemer.toLowerCase() !== NEG_RISK_ADAPTER_LOWER) {
    context.Redemption.set({
      id: getEventKey(event.chainId, event.block.number, event.logIndex),
      timestamp: BigInt(event.block.timestamp),
      redeemer,
      condition: conditionId,
      indexSets: event.params.indexSets.map((v: bigint) => v),
      payout: event.params.payout,
    });
  }

  // OI: Only track USDC redemptions
  if (collateralToken.toLowerCase() === USDC_LOWER) {
    await updateOpenInterest(context, conditionId, -event.params.payout);
  }

  // PnL: Redeem = sell at payout price (skip NRA — handled there)
  if (redeemer.toLowerCase() !== NEG_RISK_ADAPTER_LOWER) {
    if (condition.payoutDenominator === 0n) return;

    const payoutNumerators = condition.payoutNumerators;
    const payoutDenominator = condition.payoutDenominator;
    const positionIds = condition.positionIds;

    for (let i = 0; i < 2; i++) {
      const userPosition = await loadOrCreateUserPosition(
        context,
        redeemer,
        positionIds[i]!,
      );
      const amount = userPosition.amount;
      const price =
        (payoutNumerators[i]! * COLLATERAL_SCALE) / payoutDenominator;
      await updateUserPositionWithSell(
        context,
        redeemer,
        positionIds[i]!,
        price,
        amount,
      );
    }
  }
});
```

## File: src/handlers/Exchange.ts
```typescript
import { Exchange, type Orderbook, type OrdersMatchedGlobal } from "generated";
import {
  parseOrderFilled,
  updateUserPositionWithBuy,
  updateUserPositionWithSell,
} from "../utils/pnl.js";
import { COLLATERAL_SCALE } from "../utils/constants.js";
import { getMarketMetadata } from "../effects/marketMetadata.js";

const TRADE_TYPE_BUY = "Buy";
const TRADE_TYPE_SELL = "Sell";
const COLLATERAL_SCALE_DEC = 1_000_000;

function getOrderSide(makerAssetId: bigint): string {
  return makerAssetId === 0n ? TRADE_TYPE_BUY : TRADE_TYPE_SELL;
}

function getOrderSize(
  makerAmountFilled: bigint,
  takerAmountFilled: bigint,
  side: string,
): bigint {
  return side === TRADE_TYPE_BUY ? makerAmountFilled : takerAmountFilled;
}

function scaleBigInt(value: bigint): number {
  return Number(value) / COLLATERAL_SCALE_DEC;
}

async function getOrCreateOrderbook(
  context: any,
  tokenId: string,
): Promise<Orderbook> {
  const existing = await context.Orderbook.get(tokenId);
  if (existing) return existing;
  return {
    id: tokenId,
    tradesQuantity: 0n,
    buysQuantity: 0n,
    sellsQuantity: 0n,
    collateralVolume: 0n,
    scaledCollateralVolume: 0,
    collateralBuyVolume: 0n,
    scaledCollateralBuyVolume: 0,
    collateralSellVolume: 0n,
    scaledCollateralSellVolume: 0,
  };
}

async function getOrCreateGlobal(
  context: any,
): Promise<OrdersMatchedGlobal> {
  const existing = await context.OrdersMatchedGlobal.get("");
  if (existing) return existing;
  return {
    id: "",
    tradesQuantity: 0n,
    buysQuantity: 0n,
    sellsQuantity: 0n,
    collateralVolume: 0n,
    scaledCollateralVolume: 0,
    collateralBuyVolume: 0n,
    scaledCollateralBuyVolume: 0,
    collateralSellVolume: 0n,
    scaledCollateralSellVolume: 0,
  };
}

// ============================================================
// OrderFilled — individual order fill records + orderbook updates
// ============================================================

Exchange.OrderFilled.handler(async ({ event, context }) => {
  const makerAssetId = event.params.makerAssetId;
  const takerAssetId = event.params.takerAssetId;
  const side = getOrderSide(makerAssetId);
  const size = getOrderSize(
    event.params.makerAmountFilled,
    event.params.takerAmountFilled,
    side,
  );

  const tokenId =
    side === TRADE_TYPE_BUY ? takerAssetId.toString() : makerAssetId.toString();

  // Record OrderFilledEvent
  const eventId = `${event.chainId}_${event.block.number}_${event.logIndex}`;
  context.OrderFilledEvent.set({
    id: eventId,
    transactionHash: event.transaction.hash,
    timestamp: BigInt(event.block.timestamp),
    orderHash: event.params.orderHash,
    maker: event.params.maker,
    taker: event.params.taker,
    makerAssetId: makerAssetId.toString(),
    takerAssetId: takerAssetId.toString(),
    makerAmountFilled: event.params.makerAmountFilled,
    takerAmountFilled: event.params.takerAmountFilled,
    fee: event.params.fee,
  });

  // Update Orderbook
  const orderbook = await getOrCreateOrderbook(context, tokenId);
  const newVolume = orderbook.collateralVolume + size;

  if (side === TRADE_TYPE_BUY) {
    const newBuyVol = orderbook.collateralBuyVolume + size;
    context.Orderbook.set({
      ...orderbook,
      collateralVolume: newVolume,
      scaledCollateralVolume: scaleBigInt(newVolume),
      tradesQuantity: orderbook.tradesQuantity + 1n,
      buysQuantity: orderbook.buysQuantity + 1n,
      collateralBuyVolume: newBuyVol,
      scaledCollateralBuyVolume: scaleBigInt(newBuyVol),
    });
  } else {
    const newSellVol = orderbook.collateralSellVolume + size;
    context.Orderbook.set({
      ...orderbook,
      collateralVolume: newVolume,
      scaledCollateralVolume: scaleBigInt(newVolume),
      tradesQuantity: orderbook.tradesQuantity + 1n,
      sellsQuantity: orderbook.sellsQuantity + 1n,
      collateralSellVolume: newSellVol,
      scaledCollateralSellVolume: scaleBigInt(newSellVol),
    });
  }

  // PnL: Update user position based on order fill
  const order = parseOrderFilled(event.params);
  const price =
    order.baseAmount > 0n
      ? (order.quoteAmount * COLLATERAL_SCALE) / order.baseAmount
      : 0n;

  if (order.side === "BUY") {
    await updateUserPositionWithBuy(
      context,
      order.account,
      order.positionId,
      price,
      order.baseAmount,
    );
  } else {
    await updateUserPositionWithSell(
      context,
      order.account,
      order.positionId,
      price,
      order.baseAmount,
    );
  }
});

// ============================================================
// OrdersMatched — batch match records + global volume
// ============================================================

Exchange.OrdersMatched.handler(async ({ event, context }) => {
  // Note: In the original subgraph, amounts are swapped for OrdersMatched
  const makerAmountFilled = event.params.takerAmountFilled;
  const takerAmountFilled = event.params.makerAmountFilled;
  const side = getOrderSide(event.params.makerAssetId);
  const size = getOrderSize(makerAmountFilled, takerAmountFilled, side);

  // Record OrdersMatchedEvent
  context.OrdersMatchedEvent.set({
    id: `${event.chainId}_${event.block.number}_${event.logIndex}`,
    timestamp: BigInt(event.block.timestamp),
    makerAssetID: event.params.makerAssetId,
    takerAssetID: event.params.takerAssetId,
    makerAmountFilled: event.params.makerAmountFilled,
    takerAmountFilled: event.params.takerAmountFilled,
  });

  // Update global volume
  const global = await getOrCreateGlobal(context);
  const newVolume = global.collateralVolume + size;

  if (side === TRADE_TYPE_BUY) {
    const newBuyVol = global.collateralBuyVolume + size;
    context.OrdersMatchedGlobal.set({
      ...global,
      tradesQuantity: global.tradesQuantity + 1n,
      collateralVolume: newVolume,
      scaledCollateralVolume: scaleBigInt(newVolume),
      buysQuantity: global.buysQuantity + 1n,
      collateralBuyVolume: newBuyVol,
      scaledCollateralBuyVolume: scaleBigInt(newBuyVol),
    });
  } else {
    const newSellVol = global.collateralSellVolume + size;
    context.OrdersMatchedGlobal.set({
      ...global,
      tradesQuantity: global.tradesQuantity + 1n,
      collateralVolume: newVolume,
      scaledCollateralVolume: scaleBigInt(newVolume),
      sellsQuantity: global.sellsQuantity + 1n,
      collateralSellVolume: newSellVol,
      scaledCollateralSellVolume: scaleBigInt(newSellVol),
    });
  }
});

// ============================================================
// TokenRegistered — link token IDs to conditions
// ============================================================

Exchange.TokenRegistered.handler(async ({ event, context }) => {
  const token0Str = event.params.token0.toString();
  const token1Str = event.params.token1.toString();
  const condition = event.params.conditionId;

  // Fetch market metadata from Polymarket Gamma API (cached + rate-limited)
  const metadata = await context.effect(getMarketMetadata, token0Str);
  const marketName = metadata?.question ?? "";
  const marketSlug = metadata?.slug ?? "";
  const outcomes = metadata?.outcomes ?? "[]";
  const description = metadata?.description ?? "";
  const image = metadata?.image ?? "";
  const startDate = metadata?.startDate ?? "";
  const endDate = metadata?.endDate ?? "";

  const data0 = await context.MarketData.get(token0Str);
  if (!data0) {
    context.MarketData.set({
      id: token0Str,
      condition,
      outcomeIndex: undefined,
      marketName,
      marketSlug,
      outcomes,
      description,
      image,
      startDate,
      endDate,
    });
  }

  const data1 = await context.MarketData.get(token1Str);
  if (!data1) {
    context.MarketData.set({
      id: token1Str,
      condition,
      outcomeIndex: undefined,
      marketName,
      marketSlug,
      outcomes,
      description,
      image,
      startDate,
      endDate,
    });
  }
});
```

## File: src/handlers/FeeModule.ts
```typescript
import { FeeModule } from "generated";
import { NEG_RISK_FEE_MODULE } from "../utils/constants.js";

FeeModule.FeeRefunded.handler(async ({ event, context }) => {
  const negRisk =
    event.srcAddress.toLowerCase() === NEG_RISK_FEE_MODULE.toLowerCase();

  context.FeeRefunded.set({
    id: `${event.chainId}_${event.block.number}_${event.logIndex}`,
    orderHash: event.params.orderHash,
    tokenId: event.params.id.toString(),
    timestamp: BigInt(event.block.timestamp),
    refundee: event.params.to,
    feeRefunded: event.params.refund,
    feeCharged: event.params.feeCharged,
    negRisk,
  });
});
```

## File: src/handlers/FixedProductMarketMaker.ts
```typescript
import { FixedProductMarketMaker } from "generated";
import { COLLATERAL_SCALE } from "../utils/constants.js";
import {
  nthRoot,
  calculatePrices,
  scaleBigInt,
  maxBigInt,
  timestampToDay,
  ADDRESS_ZERO,
} from "../utils/fpmm.js";
import {
  updateUserPositionWithBuy,
  updateUserPositionWithSell,
  computeFpmmPrice,
} from "../utils/pnl.js";
import { getEventKey } from "../utils/negRisk.js";

const COLLATERAL_SCALE_DEC = 1_000_000;

// ============================================================
// Helper: load pool membership
// ============================================================

async function loadPoolMembership(
  context: any,
  poolId: string,
  funder: string,
): Promise<{ id: string; pool_id: string; funder: string; amount: bigint }> {
  const id = `${poolId}-${funder}`;
  const existing = await context.FpmmPoolMembership.get(id);
  if (existing) return existing;
  return { id, pool_id: poolId, funder, amount: 0n };
}

// ============================================================
// FPMMBuy — FPMM metrics + PnL + transaction record
// ============================================================

FixedProductMarketMaker.FPMMBuy.handler(async ({ event, context }) => {
  const fpmmAddress = event.srcAddress;
  const fpmm = await context.FixedProductMarketMaker.get(fpmmAddress);
  if (!fpmm) return;

  // Update outcome token amounts
  const oldAmounts = fpmm.outcomeTokenAmounts;
  const investmentMinusFees =
    event.params.investmentAmount - event.params.feeAmount;
  const outcomeIndex = Number(event.params.outcomeIndex);

  const newAmounts: bigint[] = [];
  let amountsProduct = 1n;
  for (let i = 0; i < oldAmounts.length; i++) {
    let newAmt: bigint;
    if (i === outcomeIndex) {
      newAmt =
        oldAmounts[i]! + investmentMinusFees - event.params.outcomeTokensBought;
    } else {
      newAmt = oldAmounts[i]! + investmentMinusFees;
    }
    newAmounts.push(newAmt);
    amountsProduct *= newAmt;
  }

  const liquidityParameter = nthRoot(amountsProduct, newAmounts.length);
  const newVolume = fpmm.collateralVolume + event.params.investmentAmount;
  const newBuyVol = fpmm.collateralBuyVolume + event.params.investmentAmount;
  const newFeeVol = fpmm.feeVolume + event.params.feeAmount;

  context.FixedProductMarketMaker.set({
    ...fpmm,
    outcomeTokenAmounts: newAmounts,
    outcomeTokenPrices: calculatePrices(newAmounts),
    liquidityParameter,
    scaledLiquidityParameter: scaleBigInt(liquidityParameter),
    collateralVolume: newVolume,
    scaledCollateralVolume: scaleBigInt(newVolume),
    collateralBuyVolume: newBuyVol,
    scaledCollateralBuyVolume: scaleBigInt(newBuyVol),
    feeVolume: newFeeVol,
    scaledFeeVolume: scaleBigInt(newFeeVol),
    lastActiveDay: timestampToDay(event.block.timestamp),
    tradesQuantity: fpmm.tradesQuantity + 1n,
    buysQuantity: fpmm.buysQuantity + 1n,
  });

  // Record transaction
  context.FpmmTransaction.set({
    id: getEventKey(event.chainId, event.block.number, event.logIndex),
    type: "Buy",
    timestamp: BigInt(event.block.timestamp),
    market_id: fpmmAddress,
    user: event.params.buyer,
    tradeAmount: event.params.investmentAmount,
    feeAmount: event.params.feeAmount,
    outcomeIndex: event.params.outcomeIndex,
    outcomeTokensAmount: event.params.outcomeTokensBought,
  });

  // PnL: Buy outcome token
  if (event.params.outcomeTokensBought > 0n) {
    const price =
      (event.params.investmentAmount * COLLATERAL_SCALE) /
      event.params.outcomeTokensBought;

    // Look up condition from FPMM
    const conditionId = fpmm.conditions[0];
    if (conditionId) {
      const condition = await context.Condition.get(conditionId);
      if (condition) {
        const positionId = condition.positionIds[outcomeIndex];
        if (positionId !== undefined) {
          await updateUserPositionWithBuy(
            context,
            event.params.buyer,
            positionId,
            price,
            event.params.outcomeTokensBought,
          );
        }
      }
    }
  }
});

// ============================================================
// FPMMSell — FPMM metrics + PnL + transaction record
// ============================================================

FixedProductMarketMaker.FPMMSell.handler(async ({ event, context }) => {
  const fpmmAddress = event.srcAddress;
  const fpmm = await context.FixedProductMarketMaker.get(fpmmAddress);
  if (!fpmm) return;

  // Update outcome token amounts
  const oldAmounts = fpmm.outcomeTokenAmounts;
  const returnPlusFees = event.params.returnAmount + event.params.feeAmount;
  const outcomeIndex = Number(event.params.outcomeIndex);

  const newAmounts: bigint[] = [];
  let amountsProduct = 1n;
  for (let i = 0; i < oldAmounts.length; i++) {
    let newAmt: bigint;
    if (i === outcomeIndex) {
      newAmt =
        oldAmounts[i]! - returnPlusFees + event.params.outcomeTokensSold;
    } else {
      newAmt = oldAmounts[i]! - returnPlusFees;
    }
    newAmounts.push(newAmt);
    amountsProduct *= newAmt;
  }

  const liquidityParameter = nthRoot(amountsProduct, newAmounts.length);
  const newVolume = fpmm.collateralVolume + event.params.returnAmount;
  const newSellVol = fpmm.collateralSellVolume + event.params.returnAmount;
  const newFeeVol = fpmm.feeVolume + event.params.feeAmount;

  context.FixedProductMarketMaker.set({
    ...fpmm,
    outcomeTokenAmounts: newAmounts,
    outcomeTokenPrices: calculatePrices(newAmounts),
    liquidityParameter,
    scaledLiquidityParameter: scaleBigInt(liquidityParameter),
    collateralVolume: newVolume,
    scaledCollateralVolume: scaleBigInt(newVolume),
    collateralSellVolume: newSellVol,
    scaledCollateralSellVolume: scaleBigInt(newSellVol),
    feeVolume: newFeeVol,
    scaledFeeVolume: scaleBigInt(newFeeVol),
    lastActiveDay: timestampToDay(event.block.timestamp),
    tradesQuantity: fpmm.tradesQuantity + 1n,
    sellsQuantity: fpmm.sellsQuantity + 1n,
  });

  // Record transaction
  context.FpmmTransaction.set({
    id: getEventKey(event.chainId, event.block.number, event.logIndex),
    type: "Sell",
    timestamp: BigInt(event.block.timestamp),
    market_id: fpmmAddress,
    user: event.params.seller,
    tradeAmount: event.params.returnAmount,
    feeAmount: event.params.feeAmount,
    outcomeIndex: event.params.outcomeIndex,
    outcomeTokensAmount: event.params.outcomeTokensSold,
  });

  // PnL: Sell outcome token
  if (event.params.outcomeTokensSold > 0n) {
    const price =
      (event.params.returnAmount * COLLATERAL_SCALE) /
      event.params.outcomeTokensSold;

    const conditionId = fpmm.conditions[0];
    if (conditionId) {
      const condition = await context.Condition.get(conditionId);
      if (condition) {
        const positionId = condition.positionIds[outcomeIndex];
        if (positionId !== undefined) {
          await updateUserPositionWithSell(
            context,
            event.params.seller,
            positionId,
            price,
            event.params.outcomeTokensSold,
          );
        }
      }
    }
  }
});

// ============================================================
// FPMMFundingAdded — FPMM metrics + PnL + record
// ============================================================

FixedProductMarketMaker.FPMMFundingAdded.handler(async ({ event, context }) => {
  const fpmmAddress = event.srcAddress;
  const fpmm = await context.FixedProductMarketMaker.get(fpmmAddress);
  if (!fpmm) return;

  // Update outcome token amounts
  const oldAmounts = fpmm.outcomeTokenAmounts;
  const amountsAdded = event.params.amountsAdded;
  const newAmounts: bigint[] = [];
  let amountsProduct = 1n;
  for (let i = 0; i < oldAmounts.length; i++) {
    const newAmt = oldAmounts[i]! + (amountsAdded[i] ?? 0n);
    newAmounts.push(newAmt);
    amountsProduct *= newAmt;
  }

  const liquidityParameter = nthRoot(amountsProduct, newAmounts.length);
  const newTotalSupply = fpmm.totalSupply + event.params.sharesMinted;

  // Update prices only on first liquidity addition
  const newPrices =
    fpmm.totalSupply === 0n
      ? calculatePrices(newAmounts)
      : fpmm.outcomeTokenPrices;

  context.FixedProductMarketMaker.set({
    ...fpmm,
    outcomeTokenAmounts: newAmounts,
    outcomeTokenPrices: newPrices,
    liquidityParameter,
    scaledLiquidityParameter: scaleBigInt(liquidityParameter),
    totalSupply: newTotalSupply,
    liquidityAddQuantity: fpmm.liquidityAddQuantity + 1n,
  });

  // Compute amountsRefunded
  const addedFunds = maxBigInt(amountsAdded);
  const amountsRefunded: bigint[] = [];
  for (let i = 0; i < amountsAdded.length; i++) {
    amountsRefunded.push(addedFunds - (amountsAdded[i] ?? 0n));
  }

  // Record funding addition
  context.FpmmFundingAddition.set({
    id: getEventKey(event.chainId, event.block.number, event.logIndex),
    timestamp: BigInt(event.block.timestamp),
    fpmm_id: fpmmAddress,
    funder: event.params.funder,
    amountsAdded: amountsAdded.map((v: bigint) => v),
    amountsRefunded,
    sharesMinted: event.params.sharesMinted,
  });

  // PnL: Funding added = buy sendback token + buy LP shares
  const conditionId = fpmm.conditions[0];
  if (!conditionId) return;
  const condition = await context.Condition.get(conditionId);
  if (!condition) return;

  const totalAdded = (amountsAdded[0] ?? 0n) + (amountsAdded[1] ?? 0n);
  if (totalAdded === 0n) return;

  // Sendback: the cheaper outcome gets refunded to the user
  const outcomeIndex =
    (amountsAdded[0] ?? 0n) > (amountsAdded[1] ?? 0n) ? 1 : 0;
  const sendbackAmount =
    (amountsAdded[1 - outcomeIndex] ?? 0n) - (amountsAdded[outcomeIndex] ?? 0n);

  let tokenCost = 0n;
  if (sendbackAmount > 0n) {
    const sendbackPrice = computeFpmmPrice(amountsAdded, outcomeIndex);
    const positionId = condition.positionIds[outcomeIndex];
    if (positionId !== undefined) {
      await updateUserPositionWithBuy(
        context,
        event.params.funder,
        positionId,
        sendbackPrice,
        sendbackAmount,
      );
    }
    tokenCost = (sendbackAmount * sendbackPrice) / COLLATERAL_SCALE;
  }

  // Buy LP shares (always tracked, even for balanced additions)
  if (event.params.sharesMinted > 0n) {
    const totalUSDCSpend = maxBigInt(amountsAdded);
    const lpShareCost = totalUSDCSpend - tokenCost;
    const lpSharePrice =
      (lpShareCost * COLLATERAL_SCALE) / event.params.sharesMinted;

    const fpmmAsBigInt = BigInt(fpmmAddress);
    await updateUserPositionWithBuy(
      context,
      event.params.funder,
      fpmmAsBigInt,
      lpSharePrice,
      event.params.sharesMinted,
    );
  }
});

// ============================================================
// FPMMFundingRemoved — FPMM metrics + PnL + record
// ============================================================

FixedProductMarketMaker.FPMMFundingRemoved.handler(
  async ({ event, context }) => {
    const fpmmAddress = event.srcAddress;
    const fpmm = await context.FixedProductMarketMaker.get(fpmmAddress);
    if (!fpmm) return;

    // Update outcome token amounts
    const oldAmounts = fpmm.outcomeTokenAmounts;
    const amountsRemoved = event.params.amountsRemoved;
    const newAmounts: bigint[] = [];
    let amountsProduct = 1n;
    for (let i = 0; i < oldAmounts.length; i++) {
      const newAmt = oldAmounts[i]! - (amountsRemoved[i] ?? 0n);
      newAmounts.push(newAmt);
      amountsProduct *= newAmt;
    }

    const liquidityParameter = nthRoot(amountsProduct, newAmounts.length);
    const newTotalSupply = fpmm.totalSupply - event.params.sharesBurnt;

    // Zero out prices if all liquidity removed
    const newPrices =
      newTotalSupply === 0n
        ? calculatePrices(newAmounts)
        : fpmm.outcomeTokenPrices;

    context.FixedProductMarketMaker.set({
      ...fpmm,
      outcomeTokenAmounts: newAmounts,
      outcomeTokenPrices: newPrices,
      liquidityParameter,
      scaledLiquidityParameter: scaleBigInt(liquidityParameter),
      totalSupply: newTotalSupply,
      liquidityRemoveQuantity: fpmm.liquidityRemoveQuantity + 1n,
    });

    // Record funding removal
    context.FpmmFundingRemoval.set({
      id: getEventKey(event.chainId, event.block.number, event.logIndex),
      timestamp: BigInt(event.block.timestamp),
      fpmm_id: fpmmAddress,
      funder: event.params.funder,
      amountsRemoved: amountsRemoved.map((v: bigint) => v),
      collateralRemoved: event.params.collateralRemovedFromFeePool,
      sharesBurnt: event.params.sharesBurnt,
    });

    // PnL: Funding removed = buy tokens at market price + sell LP shares
    const conditionId = fpmm.conditions[0];
    if (!conditionId) return;
    const condition = await context.Condition.get(conditionId);
    if (!condition) return;

    const totalRemoved =
      (amountsRemoved[0] ?? 0n) + (amountsRemoved[1] ?? 0n);
    if (totalRemoved === 0n) return;

    let tokensCost = 0n;
    for (let i = 0; i < 2; i++) {
      const positionId = condition.positionIds[i];
      if (positionId === undefined) continue;
      const tokenPrice = computeFpmmPrice(amountsRemoved, i);
      const tokenAmount = amountsRemoved[i] ?? 0n;
      tokensCost += (tokenPrice * tokenAmount) / COLLATERAL_SCALE;

      await updateUserPositionWithBuy(
        context,
        event.params.funder,
        positionId,
        tokenPrice,
        tokenAmount,
      );
    }

    // Sell LP shares
    if (event.params.sharesBurnt > 0n) {
      const lpSalePrice =
        ((event.params.collateralRemovedFromFeePool - tokensCost) *
          COLLATERAL_SCALE) /
        event.params.sharesBurnt;

      const fpmmAsBigInt = BigInt(fpmmAddress);
      await updateUserPositionWithSell(
        context,
        event.params.funder,
        fpmmAsBigInt,
        lpSalePrice,
        event.params.sharesBurnt,
      );
    }
  },
);

// ============================================================
// Transfer — pool share tracking
// ============================================================

FixedProductMarketMaker.Transfer.handler(async ({ event, context }) => {
  const fpmmAddress = event.srcAddress;
  const from = event.params.from;
  const to = event.params.to;
  const value = event.params.value;

  if (from !== ADDRESS_ZERO) {
    const fromMembership = await loadPoolMembership(context, fpmmAddress, from);
    context.FpmmPoolMembership.set({
      ...fromMembership,
      amount: fromMembership.amount - value,
    });
  }

  if (to !== ADDRESS_ZERO) {
    const toMembership = await loadPoolMembership(context, fpmmAddress, to);
    context.FpmmPoolMembership.set({
      ...toMembership,
      amount: toMembership.amount + value,
    });
  }
});
```

## File: src/handlers/FPMMFactory.ts
```typescript
import { FPMMFactory } from "generated";
import { CONDITIONAL_TOKENS } from "../utils/constants.js";
import { timestampToDay } from "../utils/fpmm.js";

const CONDITIONAL_TOKENS_LOWER = CONDITIONAL_TOKENS.toLowerCase();

// ============================================================
// contractRegister — register dynamic FPMM addresses (MUST be before handler)
// ============================================================

FPMMFactory.FixedProductMarketMakerCreation.contractRegister(
  ({ event, context }) => {
    context.addFixedProductMarketMaker(event.params.fixedProductMarketMaker);
  },
);

// ============================================================
// FixedProductMarketMakerCreation — create FPMM entity
// ============================================================

FPMMFactory.FixedProductMarketMakerCreation.handler(
  async ({ event, context }) => {
    const fpmmAddress = event.params.fixedProductMarketMaker;
    const conditionalTokensAddress =
      event.params.conditionalTokens.toLowerCase();

    // Only index FPMMs using our ConditionalTokens
    if (conditionalTokensAddress !== CONDITIONAL_TOKENS_LOWER) return;

    const conditionIds = event.params.conditionIds.map((id: string) => id);

    // Verify all conditions exist
    for (const conditionId of conditionIds) {
      const condition = await context.Condition.get(conditionId);
      if (!condition) return;
    }

    const outcomeSlotCount = 2; // Polymarket uses binary conditions

    context.FixedProductMarketMaker.set({
      id: fpmmAddress,
      creator: event.params.creator,
      creationTimestamp: BigInt(event.block.timestamp),
      creationTransactionHash: event.transaction.hash,
      collateralToken: event.params.collateralToken,
      conditionalTokenAddress: conditionalTokensAddress,
      conditions: conditionIds,
      fee: event.params.fee,
      outcomeSlotCount: BigInt(outcomeSlotCount),
      // Zero-initialized metrics
      totalSupply: 0n,
      outcomeTokenAmounts: Array(outcomeSlotCount).fill(0n),
      outcomeTokenPrices: Array(outcomeSlotCount).fill(0),
      lastActiveDay: timestampToDay(event.block.timestamp),
      collateralVolume: 0n,
      scaledCollateralVolume: 0,
      collateralBuyVolume: 0n,
      scaledCollateralBuyVolume: 0,
      collateralSellVolume: 0n,
      scaledCollateralSellVolume: 0,
      liquidityParameter: 0n,
      scaledLiquidityParameter: 0,
      feeVolume: 0n,
      scaledFeeVolume: 0,
      tradesQuantity: 0n,
      buysQuantity: 0n,
      sellsQuantity: 0n,
      liquidityAddQuantity: 0n,
      liquidityRemoveQuantity: 0n,
    });
  },
);
```

## File: src/handlers/NegRiskAdapter.ts
```typescript
import { NegRiskAdapter } from "generated";
import {
  NEG_RISK_ADAPTER,
  NEG_RISK_EXCHANGE,
  COLLATERAL_SCALE,
  FIFTY_CENTS,
} from "../utils/constants.js";
import {
  getEventKey,
  getNegRiskQuestionId,
  getConditionId,
  getNegRiskPositionId,
  indexSetContains,
} from "../utils/negRisk.js";
import {
  updateUserPositionWithBuy,
  updateUserPositionWithSell,
  loadOrCreateUserPosition,
  computeNegRiskYesPrice,
} from "../utils/pnl.js";

const NEG_RISK_EXCHANGE_LOWER = NEG_RISK_EXCHANGE.toLowerCase();
const FEE_DENOMINATOR = 10_000n;
const YES_INDEX = 0;
const NO_INDEX = 1;

// ============================================================
// Helper: get or create OI entities
// ============================================================

async function getOrCreateMarketOI(
  context: any,
  conditionId: string,
): Promise<{ id: string; amount: bigint }> {
  const existing = await context.MarketOpenInterest.get(conditionId);
  if (existing) return existing;
  return { id: conditionId, amount: 0n };
}

async function getOrCreateGlobalOI(
  context: any,
): Promise<{ id: string; amount: bigint }> {
  const existing = await context.GlobalOpenInterest.get("");
  if (existing) return existing;
  return { id: "", amount: 0n };
}

async function updateMarketOI(
  context: any,
  conditionId: string,
  amount: bigint,
): Promise<void> {
  const marketOI = await getOrCreateMarketOI(context, conditionId);
  context.MarketOpenInterest.set({
    ...marketOI,
    amount: marketOI.amount + amount,
  });
}

async function updateGlobalOI(
  context: any,
  amount: bigint,
): Promise<void> {
  const globalOI = await getOrCreateGlobalOI(context);
  context.GlobalOpenInterest.set({
    ...globalOI,
    amount: globalOI.amount + amount,
  });
}

async function updateOpenInterest(
  context: any,
  conditionId: string,
  amount: bigint,
): Promise<void> {
  await updateMarketOI(context, conditionId, amount);
  await updateGlobalOI(context, amount);
}

// ============================================================
// MarketPrepared — create NegRiskEvent
// ============================================================

NegRiskAdapter.MarketPrepared.handler(async ({ event, context }) => {
  context.NegRiskEvent.set({
    id: event.params.marketId,
    feeBps: event.params.feeBips,
    questionCount: 0n,
  });
});

// ============================================================
// QuestionPrepared — increment NegRiskEvent questionCount
// ============================================================

NegRiskAdapter.QuestionPrepared.handler(async ({ event, context }) => {
  const negRiskEvent = await context.NegRiskEvent.get(event.params.marketId);
  if (!negRiskEvent) return;

  context.NegRiskEvent.set({
    ...negRiskEvent,
    questionCount: negRiskEvent.questionCount + 1n,
  });
});

// ============================================================
// PositionSplit — Activity + OI + PnL
// ============================================================

NegRiskAdapter.PositionSplit.handler(async ({ event, context }) => {
  const conditionId = event.params.conditionId;
  const stakeholder = event.params.stakeholder;
  const skipExchange = stakeholder.toLowerCase() === NEG_RISK_EXCHANGE_LOWER;

  // OI: Check condition exists
  const condition = await context.Condition.get(conditionId);
  if (condition) {
    await updateOpenInterest(context, conditionId, event.params.amount);
  }

  // Activity: Create Split (skip NegRiskExchange)
  if (!skipExchange) {
    context.Split.set({
      id: getEventKey(event.chainId, event.block.number, event.logIndex),
      timestamp: BigInt(event.block.timestamp),
      stakeholder,
      condition: conditionId,
      amount: event.params.amount,
    });
  }

  // PnL: Split = buying both outcomes at 50 cents each (skip NegRiskExchange)
  if (!skipExchange && condition) {
    const positionIds = condition.positionIds;
    for (let i = 0; i < 2; i++) {
      await updateUserPositionWithBuy(
        context,
        stakeholder,
        positionIds[i]!,
        FIFTY_CENTS,
        event.params.amount,
      );
    }
  }
});

// ============================================================
// PositionsMerge — Activity + OI + PnL
// ============================================================

NegRiskAdapter.PositionsMerge.handler(async ({ event, context }) => {
  const conditionId = event.params.conditionId;
  const stakeholder = event.params.stakeholder;
  const skipExchange = stakeholder.toLowerCase() === NEG_RISK_EXCHANGE_LOWER;

  // OI: Check condition exists
  const condition = await context.Condition.get(conditionId);
  if (condition) {
    await updateOpenInterest(context, conditionId, -event.params.amount);
  }

  // Activity: Create Merge (skip NegRiskExchange)
  if (!skipExchange) {
    context.Merge.set({
      id: getEventKey(event.chainId, event.block.number, event.logIndex),
      timestamp: BigInt(event.block.timestamp),
      stakeholder,
      condition: conditionId,
      amount: event.params.amount,
    });
  }

  // PnL: Merge = selling both outcomes at 50 cents each (skip NegRiskExchange)
  if (!skipExchange && condition) {
    const positionIds = condition.positionIds;
    for (let i = 0; i < 2; i++) {
      await updateUserPositionWithSell(
        context,
        stakeholder,
        positionIds[i]!,
        FIFTY_CENTS,
        event.params.amount,
      );
    }
  }
});

// ============================================================
// PayoutRedemption — Activity + OI + PnL
// ============================================================

NegRiskAdapter.PayoutRedemption.handler(async ({ event, context }) => {
  const conditionId = event.params.conditionId;

  // OI: Check condition exists
  const condition = await context.Condition.get(conditionId);
  if (condition) {
    await updateOpenInterest(context, conditionId, -event.params.payout);
  }

  // Activity: Create Redemption with default indexSets for binary
  context.Redemption.set({
    id: getEventKey(event.chainId, event.block.number, event.logIndex),
    timestamp: BigInt(event.block.timestamp),
    redeemer: event.params.redeemer,
    condition: conditionId,
    indexSets: [1n, 2n],
    payout: event.params.payout,
  });

  // PnL: Sell at payout price for each outcome
  if (condition && condition.payoutDenominator > 0n) {
    const payoutNumerators = condition.payoutNumerators;
    const payoutDenominator = condition.payoutDenominator;
    const positionIds = condition.positionIds;

    for (let i = 0; i < 2; i++) {
      const amount = event.params.amounts[i]!;
      const price =
        (payoutNumerators[i]! * COLLATERAL_SCALE) / payoutDenominator;
      await updateUserPositionWithSell(
        context,
        event.params.redeemer,
        positionIds[i]!,
        price,
        amount,
      );
    }
  }
});

// ============================================================
// PositionsConverted — Activity + OI + PnL
// ============================================================

NegRiskAdapter.PositionsConverted.handler(async ({ event, context }) => {
  const marketId = event.params.marketId;
  const negRiskEvent = await context.NegRiskEvent.get(marketId);
  if (!negRiskEvent) return;

  const questionCount = Number(negRiskEvent.questionCount);
  const indexSet = event.params.indexSet;
  const stakeholder = event.params.stakeholder;

  // Activity: Create NegRiskConversion
  context.NegRiskConversion.set({
    id: getEventKey(event.chainId, event.block.number, event.logIndex),
    timestamp: BigInt(event.block.timestamp),
    stakeholder,
    negRiskMarketId: marketId,
    amount: event.params.amount,
    indexSet,
    questionCount: negRiskEvent.questionCount,
  });

  // Collect condition IDs for positions being converted
  const conditionIds: string[] = [];
  for (let qi = 0; qi < questionCount; qi++) {
    if (indexSetContains(indexSet, qi)) {
      const questionId = getNegRiskQuestionId(
        marketId as `0x${string}`,
        qi,
      );
      const conditionId = getConditionId(
        NEG_RISK_ADAPTER as `0x${string}`,
        questionId,
      ).toLowerCase();
      conditionIds.push(conditionId);
    }
  }

  // OI: Converts reduce OI when more than 1 no position
  const noCount = conditionIds.length;
  if (noCount > 1) {
    let amount = event.params.amount;
    const multiplier = BigInt(noCount - 1);
    const divisor = BigInt(noCount);

    if (negRiskEvent.feeBps > 0n) {
      const feeAmount = (amount * negRiskEvent.feeBps) / FEE_DENOMINATOR;
      amount = amount - feeAmount;

      const feeReleased = -(feeAmount * multiplier);
      for (let i = 0; i < noCount; i++) {
        await updateMarketOI(context, conditionIds[i]!, feeReleased / divisor);
      }
      await updateGlobalOI(context, feeReleased);
    }

    const collateralReleased = -(amount * multiplier);
    for (let i = 0; i < noCount; i++) {
      await updateMarketOI(
        context,
        conditionIds[i]!,
        collateralReleased / divisor,
      );
    }
    await updateGlobalOI(context, collateralReleased);
  }

  // PnL: Sell NO positions, buy YES positions
  let noPriceSum = 0n;
  let noCountPnl = 0;

  for (let qi = 0; qi < questionCount; qi++) {
    if (indexSetContains(indexSet, qi)) {
      noCountPnl++;
      const noPositionId = getNegRiskPositionId(
        marketId as `0x${string}`,
        qi,
        NO_INDEX,
      );
      const userPosition = await loadOrCreateUserPosition(
        context,
        stakeholder,
        noPositionId,
      );

      // Sell NO token at avg price
      await updateUserPositionWithSell(
        context,
        stakeholder,
        noPositionId,
        userPosition.avgPrice,
        event.params.amount,
      );

      noPriceSum += userPosition.avgPrice;
    }
  }

  // Buy YES tokens if not all positions are NO
  if (noCountPnl < questionCount && noCountPnl > 0) {
    const noPrice = noPriceSum / BigInt(noCountPnl);
    const yesPrice = computeNegRiskYesPrice(noPrice, noCountPnl, questionCount);

    for (let qi = 0; qi < questionCount; qi++) {
      if (!indexSetContains(indexSet, qi)) {
        const yesPositionId = getNegRiskPositionId(
          marketId as `0x${string}`,
          qi,
          YES_INDEX,
        );
        await updateUserPositionWithBuy(
          context,
          stakeholder,
          yesPositionId,
          yesPrice,
          event.params.amount,
        );
      }
    }
  }
});
```

## File: src/handlers/UmaSportsOracle.ts
```typescript
import { UmaSportsOracle } from "generated";

// State constants
const GameStateCreated = "Created";
const GameStateSettled = "Settled";
const GameStateCanceled = "Canceled";
const GameStatePaused = "Paused";
const GameStateEmergencySettled = "EmergencySettled";

const MarketStateCreated = "Created";
const MarketStateResolved = "Resolved";
const MarketStatePaused = "Paused";
const MarketStateEmergencyResolved = "EmergencyResolved";

// Enum mappers
function getMarketType(marketTypeEnum: bigint): string {
  if (marketTypeEnum === 0n) return "moneyline";
  if (marketTypeEnum === 1n) return "spreads";
  return "totals";
}

function getGameOrdering(gameOrderingEnum: bigint): string {
  return gameOrderingEnum === 0n ? "home" : "away";
}

function getMarketUnderdog(underdogEnum: bigint): string {
  return underdogEnum === 0n ? "home" : "away";
}

// ============================================================
// Game event handlers
// ============================================================

UmaSportsOracle.GameCreated.handler(async ({ event, context }) => {
  const gameId = event.params.gameId.toLowerCase();
  context.Game.set({
    id: gameId,
    ancillaryData: event.params.ancillaryData,
    ordering: getGameOrdering(event.params.ordering),
    state: GameStateCreated,
    homeScore: 0n,
    awayScore: 0n,
  });
});

UmaSportsOracle.GameSettled.handler(async ({ event, context }) => {
  const gameId = event.params.gameId.toLowerCase();
  const game = await context.Game.get(gameId);
  if (!game) {
    context.log.error(`Game not found: ${gameId}`);
    return;
  }
  context.Game.set({
    ...game,
    state: GameStateSettled,
    homeScore: event.params.home,
    awayScore: event.params.away,
  });
});

UmaSportsOracle.GameEmergencySettled.handler(async ({ event, context }) => {
  const gameId = event.params.gameId.toLowerCase();
  const game = await context.Game.get(gameId);
  if (!game) {
    context.log.error(`Game not found: ${gameId}`);
    return;
  }
  context.Game.set({
    ...game,
    state: GameStateEmergencySettled,
    homeScore: event.params.home,
    awayScore: event.params.away,
  });
});

UmaSportsOracle.GameCanceled.handler(async ({ event, context }) => {
  const gameId = event.params.gameId.toLowerCase();
  const game = await context.Game.get(gameId);
  if (!game) {
    context.log.error(`Game not found: ${gameId}`);
    return;
  }
  context.Game.set({
    ...game,
    state: GameStateCanceled,
  });
});

UmaSportsOracle.GamePaused.handler(async ({ event, context }) => {
  const gameId = event.params.gameId.toLowerCase();
  const game = await context.Game.get(gameId);
  if (!game) {
    context.log.error(`Game not found: ${gameId}`);
    return;
  }
  context.Game.set({
    ...game,
    state: GameStatePaused,
  });
});

UmaSportsOracle.GameUnpaused.handler(async ({ event, context }) => {
  const gameId = event.params.gameId.toLowerCase();
  const game = await context.Game.get(gameId);
  if (!game) {
    context.log.error(`Game not found: ${gameId}`);
    return;
  }
  context.Game.set({
    ...game,
    state: GameStateCreated, // Unpaused reverts to Created state
  });
});

// ============================================================
// Market event handlers
// ============================================================

UmaSportsOracle.MarketCreated.handler(async ({ event, context }) => {
  const marketId = event.params.marketId.toLowerCase();
  context.Market.set({
    id: marketId,
    gameId: event.params.gameId.toLowerCase(),
    state: MarketStateCreated,
    marketType: getMarketType(event.params.marketType),
    underdog: getMarketUnderdog(event.params.underdog),
    line: event.params.line,
    payouts: [],
  });
});

UmaSportsOracle.MarketResolved.handler(async ({ event, context }) => {
  const marketId = event.params.marketId.toLowerCase();
  const market = await context.Market.get(marketId);
  if (!market) {
    context.log.error(`Market not found: ${marketId}`);
    return;
  }
  context.Market.set({
    ...market,
    state: MarketStateResolved,
    payouts: event.params.payouts,
  });
});

UmaSportsOracle.MarketEmergencyResolved.handler(
  async ({ event, context }) => {
    const marketId = event.params.marketId.toLowerCase();
    const market = await context.Market.get(marketId);
    if (!market) {
      context.log.error(`Market not found: ${marketId}`);
      return;
    }
    context.Market.set({
      ...market,
      state: MarketStateEmergencyResolved,
      payouts: event.params.payouts,
    });
  },
);

UmaSportsOracle.MarketPaused.handler(async ({ event, context }) => {
  const marketId = event.params.marketId.toLowerCase();
  const market = await context.Market.get(marketId);
  if (!market) {
    context.log.error(`Market not found: ${marketId}`);
    return;
  }
  context.Market.set({
    ...market,
    state: MarketStatePaused,
  });
});

UmaSportsOracle.MarketUnpaused.handler(async ({ event, context }) => {
  const marketId = event.params.marketId.toLowerCase();
  const market = await context.Market.get(marketId);
  if (!market) {
    context.log.error(`Market not found: ${marketId}`);
    return;
  }
  context.Market.set({
    ...market,
    state: MarketStateCreated, // Unpaused reverts to Created state
  });
});
```

## File: src/handlers/Wallet.ts
```typescript
import { RelayHub, SafeProxyFactory, USDC } from "generated";
import {
  PROXY_WALLET_FACTORY,
  PROXY_WALLET_IMPLEMENTATION,
} from "../utils/constants.js";
import { computeProxyWalletAddress } from "../utils/wallet.js";

const GLOBAL_USDC_ID = "global";

// ============================================================
// RelayHub — proxy wallet creation
// ============================================================

RelayHub.TransactionRelayed.handler(async ({ event, context }) => {
  const from = event.params.from;
  const to = event.params.to;

  // Only process calls to the proxy wallet factory
  if (to.toLowerCase() !== PROXY_WALLET_FACTORY.toLowerCase()) {
    return;
  }

  const walletAddress = computeProxyWalletAddress(
    from as `0x${string}`,
    PROXY_WALLET_FACTORY as `0x${string}`,
    PROXY_WALLET_IMPLEMENTATION as `0x${string}`,
  );

  const existing = await context.Wallet.get(walletAddress);
  if (!existing) {
    context.Wallet.set({
      id: walletAddress,
      signer: from,
      type: "proxy",
      balance: 0n,
      lastTransfer: 0n,
      createdAt: BigInt(event.block.timestamp),
    });
  }
});

// ============================================================
// SafeProxyFactory — safe wallet creation
// ============================================================

SafeProxyFactory.ProxyCreation.handler(async ({ event, context }) => {
  const proxyAddress = event.params.proxy;

  const existing = await context.Wallet.get(proxyAddress);
  if (!existing) {
    context.Wallet.set({
      id: proxyAddress,
      signer: event.params.owner,
      type: "safe",
      balance: 0n,
      lastTransfer: 0n,
      createdAt: BigInt(event.block.timestamp),
    });
  }
});

// ============================================================
// USDC Transfer — wallet balance tracking
// ============================================================

USDC.Transfer.handler(async ({ event, context }) => {
  const from = event.params.from;
  const to = event.params.to;
  const amount = event.params.amount;
  const timestamp = BigInt(event.block.timestamp);

  // Check receiver
  const toWallet = await context.Wallet.get(to);
  if (toWallet) {
    context.Wallet.set({
      ...toWallet,
      balance: toWallet.balance + amount,
      lastTransfer: timestamp,
    });

    // Update global balance
    const global = await context.GlobalUSDCBalance.get(GLOBAL_USDC_ID);
    if (global) {
      context.GlobalUSDCBalance.set({
        ...global,
        balance: global.balance + amount,
      });
    } else {
      context.GlobalUSDCBalance.set({
        id: GLOBAL_USDC_ID,
        balance: amount,
      });
    }
  }

  // Check sender
  const fromWallet = await context.Wallet.get(from);
  if (fromWallet) {
    context.Wallet.set({
      ...fromWallet,
      balance: fromWallet.balance - amount,
      lastTransfer: timestamp,
    });

    // Update global balance
    const global = await context.GlobalUSDCBalance.get(GLOBAL_USDC_ID);
    if (global) {
      context.GlobalUSDCBalance.set({
        ...global,
        balance: global.balance - amount,
      });
    } else {
      context.GlobalUSDCBalance.set({
        id: GLOBAL_USDC_ID,
        balance: 0n - amount,
      });
    }
  }
});
```

## File: src/utils/constants.ts
```typescript
// Polygon (chain ID 137) contract addresses
export const CONDITIONAL_TOKENS = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
export const FPMM_FACTORY = "0x8B9805A2f595B6705e74F7310829f2d299D21522";
export const EXCHANGE = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
export const NEG_RISK_EXCHANGE = "0xC5d563A36AE78145C45a50134d48A1215220f80a";
export const NEG_RISK_ADAPTER = "0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296";
export const NEG_RISK_OPERATOR = "0x71523d0f655B41E805Cec45b17163f528B59B820";
export const NEG_RISK_WRAPPED_COLLATERAL =
  "0x3A3BD7bb9528E159577F7C2e685CC81A765002E2";
export const USDC = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
export const UMA_SPORTS_ORACLE =
  "0xb21182d0494521Cf45DbbeEbb5A3ACAAb6d22093";
export const FEE_MODULE = "0xE3f18aCc55091e2c48d883fc8C8413319d4Ab7b0";
export const NEG_RISK_FEE_MODULE =
  "0xB768891e3130F6dF18214Ac804d4DB76c2C37730";
export const RELAY_HUB = "0xD216153c06E857cD7f72665E0aF1d7D82172F494";
export const SAFE_PROXY_FACTORY =
  "0xaacFeEa03eb1561C4e67d661e40682Bd20E3541b";
export const PROXY_WALLET_FACTORY =
  "0xaB45c5A4B0c941a2F231C04C3f49182e1A254052";
export const PROXY_WALLET_IMPLEMENTATION =
  "0x44e999d5c2F66Ef0861317f9A4805AC2e90aEB4f";

export const COLLATERAL_SCALE = 10n ** 6n; // USDC 6 decimals
export const COLLATERAL_SCALE_DEC = 1_000_000;
export const FIFTY_CENTS = COLLATERAL_SCALE / 2n;

export enum TradeType {
  BUY = 0,
  SELL = 1,
}
```

## File: src/utils/ctf.ts
```typescript
import { keccak256, encodePacked, toHex, pad } from "viem";

const P =
  21888242871839275222246405745257275088696311157297823662689037894645226208583n;
const B = 3n;

const addModP = (a: bigint, b: bigint): bigint => ((a + b) % P + P) % P;
const mulModP = (a: bigint, b: bigint): bigint => ((a * b) % P + P) % P;

const powModP = (a: bigint, b: bigint): bigint => {
  let at = a;
  let bt = b;
  let result = 1n;
  at = ((at % P) + P) % P;

  while (bt > 0n) {
    if (bt & 1n) {
      result = mulModP(result, at);
    }
    at = mulModP(at, at);
    bt >>= 1n;
  }

  return result;
};

const legendreSymbol = (a: bigint): bigint => powModP(a, (P - 1n) >> 1n);

export function computeCollectionId(
  conditionId: `0x${string}`,
  outcomeIndex: number,
): `0x${string}` {
  // Build 64-byte payload: conditionId (32 bytes) + indexSet (32 bytes)
  const indexSet = 1n << BigInt(outcomeIndex);
  const indexSetHex = pad(toHex(indexSet), { size: 32 });

  const hashResult = keccak256(
    encodePacked(["bytes32", "bytes32"], [conditionId, indexSetHex]),
  );

  // Convert hash to bigint (big-endian)
  let hashBigInt = BigInt(hashResult);

  // Check if MSB is set
  const odd = (hashBigInt >> 255n) !== 0n;

  let x1 = hashBigInt;
  let yy = 0n;

  // Increment x1 until we find a point on the curve y^2 = x^3 + 3 (mod P)
  do {
    x1 = addModP(x1, 1n);
    yy = addModP(mulModP(x1, mulModP(x1, x1)), B);
  } while (legendreSymbol(yy) !== 1n);

  const oddToggle = 1n << 254n;
  if (odd) {
    if ((x1 & oddToggle) === 0n) {
      x1 = x1 + oddToggle;
    } else {
      x1 = x1 - oddToggle;
    }
  }

  return pad(toHex(x1), { size: 32 });
}

export function computePositionIdFromCollectionId(
  collateral: `0x${string}`,
  collectionId: `0x${string}`,
): bigint {
  const hash = keccak256(
    encodePacked(["address", "bytes32"], [collateral, collectionId]),
  );
  return BigInt(hash);
}

export function computePositionId(
  collateral: `0x${string}`,
  conditionId: `0x${string}`,
  outcomeIndex: number,
): bigint {
  const collectionId = computeCollectionId(conditionId, outcomeIndex);
  return computePositionIdFromCollectionId(collateral, collectionId);
}
```

## File: src/utils/fpmm.ts
```typescript
const COLLATERAL_SCALE_DEC = 1_000_000;
const ADDRESS_ZERO = "0x0000000000000000000000000000000000000000";

export { ADDRESS_ZERO };

export function timestampToDay(timestamp: number): bigint {
  return BigInt(Math.floor(timestamp / 86400));
}

/**
 * Nth root using Newton's method (integer approximation).
 * Adapted from the original AssemblyScript implementation.
 */
export function nthRoot(x: bigint, n: number): bigint {
  if (n <= 0) return 0n;
  if (x === 0n) return 0n;

  const nBig = BigInt(n);
  let root = x;
  let deltaRoot: bigint;

  do {
    let rootPowNLess1 = 1n;
    for (let i = 0; i < n - 1; i++) {
      rootPowNLess1 = rootPowNLess1 * root;
    }
    deltaRoot = (x / rootPowNLess1 - root) / nBig;
    root = root + deltaRoot;
  } while (deltaRoot < 0n);

  return root;
}

/**
 * Calculate outcome token prices from amounts.
 * price[i] = product / amounts[i] / sum(product / amounts[j] for all j)
 */
export function calculatePrices(outcomeTokenAmounts: bigint[]): number[] {
  const len = outcomeTokenAmounts.length;
  const prices = new Array<number>(len).fill(0);

  let totalBalance = 0n;
  let product = 1n;
  for (let i = 0; i < len; i++) {
    totalBalance += outcomeTokenAmounts[i]!;
    product *= outcomeTokenAmounts[i]!;
  }

  if (totalBalance === 0n) return prices;

  let denominator = 0n;
  for (let i = 0; i < len; i++) {
    denominator += product / outcomeTokenAmounts[i]!;
  }

  if (denominator === 0n) return prices;

  for (let i = 0; i < len; i++) {
    // price = (product / amounts[i]) / denominator
    const numerator = product / outcomeTokenAmounts[i]!;
    prices[i] = Number(numerator) / Number(denominator);
  }

  return prices;
}

export function scaleBigInt(value: bigint): number {
  return Number(value) / COLLATERAL_SCALE_DEC;
}

export function maxBigInt(arr: bigint[]): bigint {
  let max = 0n;
  for (const v of arr) {
    if (v > max) max = v;
  }
  return max;
}
```

## File: src/utils/negRisk.ts
```typescript
import { keccak256, encodePacked, toHex, pad } from "viem";
import {
  NEG_RISK_ADAPTER,
  NEG_RISK_WRAPPED_COLLATERAL,
} from "./constants.js";
import { computePositionId } from "./ctf.js";

export function getNegRiskQuestionId(
  marketId: `0x${string}`,
  questionIndex: number,
): `0x${string}` {
  // Replace last byte of marketId with questionIndex
  const base = marketId.slice(0, 64); // "0x" + 62 hex chars = 31 bytes
  const indexHex = questionIndex.toString(16).padStart(2, "0");
  return `${base}${indexHex}` as `0x${string}`;
}

export function getConditionId(
  oracle: `0x${string}`,
  questionId: `0x${string}`,
): `0x${string}` {
  // Build 84-byte payload: oracle (20 bytes) + questionId (32 bytes) + outcomeSlotCount=2 (32 bytes)
  const outcomeSlotCount = pad(toHex(2n), { size: 32 });
  return keccak256(
    encodePacked(
      ["address", "bytes32", "uint256"],
      [oracle, questionId, BigInt(2)],
    ),
  );
}

export function getNegRiskConditionId(
  negRiskMarketId: `0x${string}`,
  questionIndex: number,
): `0x${string}` {
  const questionId = getNegRiskQuestionId(negRiskMarketId, questionIndex);
  return getConditionId(NEG_RISK_ADAPTER as `0x${string}`, questionId);
}

export function getNegRiskPositionId(
  negRiskMarketId: `0x${string}`,
  questionIndex: number,
  outcomeIndex: number,
): bigint {
  const conditionId = getNegRiskConditionId(negRiskMarketId, questionIndex);
  return computePositionId(
    NEG_RISK_WRAPPED_COLLATERAL as `0x${string}`,
    conditionId,
    outcomeIndex,
  );
}

export function getUserPositionEntityId(
  user: string,
  tokenId: bigint,
): string {
  return `${user}-${tokenId.toString()}`;
}

export function indexSetContains(indexSet: bigint, index: number): boolean {
  return (indexSet & (1n << BigInt(index))) > 0n;
}

export function getEventKey(
  chainId: number,
  blockNumber: number,
  logIndex: number,
): string {
  return `${chainId}_${blockNumber}_${logIndex}`;
}
```

## File: src/utils/pnl.ts
```typescript
import { COLLATERAL_SCALE } from "./constants.js";

export function getUserPositionEntityId(
  user: string,
  tokenId: bigint,
): string {
  return `${user}-${tokenId.toString()}`;
}

export async function loadOrCreateUserPosition(
  context: any,
  user: string,
  tokenId: bigint,
): Promise<{
  id: string;
  user: string;
  tokenId: bigint;
  amount: bigint;
  avgPrice: bigint;
  realizedPnl: bigint;
  totalBought: bigint;
}> {
  const id = getUserPositionEntityId(user, tokenId);
  const existing = await context.UserPosition.get(id);
  if (existing) return existing;
  return {
    id,
    user,
    tokenId,
    amount: 0n,
    avgPrice: 0n,
    realizedPnl: 0n,
    totalBought: 0n,
  };
}

export async function updateUserPositionWithBuy(
  context: any,
  user: string,
  positionId: bigint,
  price: bigint,
  amount: bigint,
): Promise<void> {
  if (amount <= 0n) return;

  const userPosition = await loadOrCreateUserPosition(context, user, positionId);

  // avgPrice = (avgPrice * userAmount + price * buyAmount) / (userAmount + buyAmount)
  const numerator = userPosition.avgPrice * userPosition.amount + price * amount;
  const denominator = userPosition.amount + amount;
  const newAvgPrice = denominator > 0n ? numerator / denominator : 0n;

  context.UserPosition.set({
    ...userPosition,
    avgPrice: newAvgPrice,
    amount: userPosition.amount + amount,
    totalBought: userPosition.totalBought + amount,
  });
}

export async function updateUserPositionWithSell(
  context: any,
  user: string,
  positionId: bigint,
  price: bigint,
  amount: bigint,
): Promise<void> {
  const userPosition = await loadOrCreateUserPosition(context, user, positionId);

  // Cap at current position amount
  const adjustedAmount = amount > userPosition.amount ? userPosition.amount : amount;

  // realizedPnl += adjustedAmount * (price - avgPrice) / COLLATERAL_SCALE
  const deltaPnL = (adjustedAmount * (price - userPosition.avgPrice)) / COLLATERAL_SCALE;

  context.UserPosition.set({
    ...userPosition,
    realizedPnl: userPosition.realizedPnl + deltaPnL,
    amount: userPosition.amount - adjustedAmount,
  });
}

/**
 * Parse OrderFilled event into a PnL-relevant order structure.
 * The maker is always the user (taker is the exchange).
 */
export function parseOrderFilled(params: {
  makerAssetId: bigint;
  takerAssetId: bigint;
  makerAmountFilled: bigint;
  takerAmountFilled: bigint;
  maker: string;
}): {
  account: string;
  side: "BUY" | "SELL";
  baseAmount: bigint;
  quoteAmount: bigint;
  positionId: bigint;
} {
  const isBuy = params.makerAssetId === 0n;

  if (isBuy) {
    return {
      account: params.maker,
      side: "BUY",
      baseAmount: params.takerAmountFilled,
      quoteAmount: params.makerAmountFilled,
      positionId: params.takerAssetId,
    };
  } else {
    return {
      account: params.maker,
      side: "SELL",
      baseAmount: params.makerAmountFilled,
      quoteAmount: params.takerAmountFilled,
      positionId: params.makerAssetId,
    };
  }
}

/**
 * Compute FPMM price from outcome token amounts.
 * price[i] = amounts[1-i] * COLLATERAL_SCALE / (amounts[0] + amounts[1])
 */
export function computeFpmmPrice(amounts: bigint[], outcomeIndex: number): bigint {
  const total = amounts[0]! + amounts[1]!;
  if (total === 0n) return 0n;
  return (amounts[1 - outcomeIndex]! * COLLATERAL_SCALE) / total;
}

/**
 * Compute neg-risk YES price from average NO price.
 * yesPrice = (noPrice * noCount - COLLATERAL_SCALE * (noCount - 1)) / (questionCount - noCount)
 */
export function computeNegRiskYesPrice(
  noPrice: bigint,
  noCount: number,
  questionCount: number,
): bigint {
  const yesCount = questionCount - noCount;
  if (yesCount === 0) return 0n;
  return (
    noPrice * BigInt(noCount) -
    COLLATERAL_SCALE * BigInt(noCount - 1)
  ) / BigInt(yesCount);
}
```

## File: src/utils/wallet.ts
```typescript
import { keccak256, encodePacked, concat } from "viem";

export function computeCreate2Address(
  deployer: `0x${string}`,
  salt: `0x${string}`,
  initCodeHash: `0x${string}`,
): `0x${string}` {
  const hash = keccak256(
    concat(["0xff" as `0x${string}`, deployer, salt, initCodeHash]),
  );
  // Take last 20 bytes as address
  return `0x${hash.slice(26)}` as `0x${string}`;
}

export function generateProxyWalletBytecode(
  factory: `0x${string}`,
  implementation: `0x${string}`,
): `0x${string}` {
  const factoryHex = factory.slice(2).toLowerCase();
  const implHex = implementation.slice(2).toLowerCase();
  const bytecodeHex =
    "3d3d606380380380913d393d73" +
    factoryHex +
    "5af4602a57600080fd5b602d8060366000396000f3363d3d373d3d3d363d73" +
    implHex +
    "5af43d82803e903d91602b57fd5bf352e831dd" +
    "0000000000000000000000000000000000000000000000000000000000000020" +
    "0000000000000000000000000000000000000000000000000000000000000000";

  return `0x${bytecodeHex}` as `0x${string}`;
}

export function computeProxyWalletAddress(
  signer: `0x${string}`,
  factory: `0x${string}`,
  implementation: `0x${string}`,
): `0x${string}` {
  const salt = keccak256(encodePacked(["address"], [signer]));
  const initCode = generateProxyWalletBytecode(factory, implementation);
  const initCodeHash = keccak256(initCode);
  return computeCreate2Address(factory, salt, initCodeHash);
}
```

## File: .env.example
```
# To create or update a token visit https://envio.dev/app/api-tokens
ENVIO_API_TOKEN="<YOUR-API-TOKEN>"
```

## File: config.yaml
```yaml
# yaml-language-server: $schema=./node_modules/envio/evm.schema.json
name: polymarket-indexer
description: Unified Polymarket HyperIndex

contracts:
  # Phase 1A: Fee Module
  - name: FeeModule
    abi_file_path: ./abis/FeeModule.json
    events:
      - event: "FeeRefunded(bytes32 indexed orderHash, address indexed to, uint256 id, uint256 refund, uint256 indexed feeCharged)"

  # Phase 1B: Sports Oracle
  - name: UmaSportsOracle
    abi_file_path: ./abis/UmaSportsOracle.json
    events:
      - event: "GameCreated(bytes32 indexed gameId, uint8 ordering, bytes ancillaryData, uint256 timestamp)"
      - event: "GameSettled(bytes32 indexed gameId, uint256 indexed home, uint256 indexed away)"
      - event: "GameEmergencySettled(bytes32 indexed gameId, uint256 indexed home, uint256 indexed away)"
      - event: "GameCanceled(bytes32 indexed gameId)"
      - event: "GamePaused(bytes32 indexed gameId)"
      - event: "GameUnpaused(bytes32 indexed gameId)"
      - event: "MarketCreated(bytes32 indexed marketId, bytes32 indexed gameId, bytes32 indexed conditionId, uint8 marketType, uint8 underdog, uint256 line)"
      - event: "MarketPaused(bytes32 indexed marketId)"
      - event: "MarketUnpaused(bytes32 indexed marketId)"
      - event: "MarketResolved(bytes32 indexed marketId, uint256[] payouts)"
      - event: "MarketEmergencyResolved(bytes32 indexed marketId, uint256[] payouts)"

  # Phase 2A: Wallet
  - name: RelayHub
    abi_file_path: ./abis/RelayHub.json
    events:
      - event: "TransactionRelayed(address indexed relay, address indexed from, address indexed to, bytes4 selector, uint8 status, uint256 charge)"

  - name: SafeProxyFactory
    abi_file_path: ./abis/SafeProxyFactory.json
    events:
      - event: "ProxyCreation(address proxy, address owner)"

  - name: USDC
    abi_file_path: ./abis/ERC20.json
    events:
      - event: "Transfer(address indexed from, address indexed to, uint256 amount)"

  # Phase 2B: Orderbook
  - name: Exchange
    abi_file_path: ./abis/Exchange.json
    events:
      - event: "OrderFilled(bytes32 indexed orderHash, address indexed maker, address indexed taker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled, uint256 fee)"
      - event: "OrdersMatched(bytes32 indexed takerOrderHash, address indexed takerOrderMaker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled)"
      - event: "TokenRegistered(uint256 indexed token0, uint256 indexed token1, bytes32 indexed conditionId)"

  # Phase 3: Open Interest + Activity (extended in Phase 4 with PnL)
  - name: ConditionalTokens
    abi_file_path: ./abis/ConditionalTokens.json
    events:
      - event: "ConditionPreparation(bytes32 indexed conditionId, address indexed oracle, bytes32 indexed questionId, uint256 outcomeSlotCount)"
      - event: "ConditionResolution(bytes32 indexed conditionId, address indexed oracle, bytes32 indexed questionId, uint256 outcomeSlotCount, uint256[] payoutNumerators)"
      - event: "PositionSplit(address indexed stakeholder, address collateralToken, bytes32 indexed parentCollectionId, bytes32 indexed conditionId, uint256[] partition, uint256 amount)"
      - event: "PositionsMerge(address indexed stakeholder, address collateralToken, bytes32 indexed parentCollectionId, bytes32 indexed conditionId, uint256[] partition, uint256 amount)"
      - event: "PayoutRedemption(address indexed redeemer, address indexed collateralToken, bytes32 indexed parentCollectionId, bytes32 conditionId, uint256[] indexSets, uint256 payout)"

  - name: NegRiskAdapter
    abi_file_path: ./abis/NegRiskAdapter.json
    events:
      - event: "MarketPrepared(bytes32 indexed marketId, address indexed oracle, uint256 feeBips, bytes data)"
      - event: "QuestionPrepared(bytes32 indexed marketId, bytes32 indexed questionId, uint256 index, bytes data)"
      - event: "PositionSplit(address indexed stakeholder, bytes32 indexed conditionId, uint256 amount)"
      - event: "PositionsMerge(address indexed stakeholder, bytes32 indexed conditionId, uint256 amount)"
      - event: "PayoutRedemption(address indexed redeemer, bytes32 indexed conditionId, uint256[] amounts, uint256 payout)"
      - event: "PositionsConverted(address indexed stakeholder, bytes32 indexed marketId, uint256 indexed indexSet, uint256 amount)"

  - name: FPMMFactory
    abi_file_path: ./abis/FixedProductMarketMakerFactory.json
    events:
      - event: "FixedProductMarketMakerCreation(address indexed creator, address fixedProductMarketMaker, address indexed conditionalTokens, address indexed collateralToken, bytes32[] conditionIds, uint256 fee)"

  # Phase 4: Dynamic FPMM contracts (registered via FPMMFactory contractRegister)
  - name: FixedProductMarketMaker
    abi_file_path: ./abis/FixedProductMarketMaker.json
    events:
      - event: "FPMMBuy(address indexed buyer, uint256 investmentAmount, uint256 feeAmount, uint256 indexed outcomeIndex, uint256 outcomeTokensBought)"
      - event: "FPMMSell(address indexed seller, uint256 returnAmount, uint256 feeAmount, uint256 indexed outcomeIndex, uint256 outcomeTokensSold)"
      - event: "FPMMFundingAdded(address indexed funder, uint256[] amountsAdded, uint256 sharesMinted)"
      - event: "FPMMFundingRemoved(address indexed funder, uint256[] amountsRemoved, uint256 collateralRemovedFromFeePool, uint256 sharesBurnt)"
      - event: "Transfer(address indexed from, address indexed to, uint256 value)"

field_selection:
  transaction_fields:
    - hash
    - from
    - to

chains:
  - id: 137 # Polygon
    start_block: 3764531
    contracts:
      # Phase 1A: Fee Module (both FeeModule + NegRiskFeeModule share same ABI)
      - name: FeeModule
        address:
          - "0xE3f18aCc55091e2c48d883fc8C8413319d4Ab7b0"
          - "0xB768891e3130F6dF18214Ac804d4DB76c2C37730"
        start_block: 75253526

      # Phase 1B: Sports Oracle
      - name: UmaSportsOracle
        address: "0xb21182d0494521Cf45DbbeEbb5A3ACAAb6d22093"
        start_block: 68931384

      # Phase 2A: Wallet
      - name: RelayHub
        address: "0xD216153c06E857cD7f72665E0aF1d7D82172F494"
        start_block: 3764531
      - name: SafeProxyFactory
        address: "0xaacFeEa03eb1561C4e67d661e40682Bd20E3541b"
        start_block: 19426226
      - name: USDC
        address: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174"
        start_block: 5013591

      # Phase 2B: Orderbook (Exchange + NegRiskExchange share same ABI)
      - name: Exchange
        address:
          - "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E"
          - "0xC5d563A36AE78145C45a50134d48A1215220f80a"
        start_block: 33605403

      # Phase 3: Open Interest + Activity
      - name: ConditionalTokens
        address: "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045"
        start_block: 4023686
      - name: NegRiskAdapter
        address: "0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296"
        start_block: 50505403
      - name: FPMMFactory
        address: "0x8B9805A2f595B6705e74F7310829f2d299D21522"
        start_block: 4023693

      # Phase 4: Dynamic — no address, registered at runtime
      - name: FixedProductMarketMaker
```

## File: package.json
```json
{
  "name": "./polymarket-indexer",
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "codegen": "envio codegen",
    "dev": "envio dev",
    "start": "envio start",
    "test": "vitest run"
  },
  "devDependencies": {
    "@types/node": "^24.10.1",
    "typescript": "5.9.3",
    "vitest": "4.0.16"
  },
  "dependencies": {
    "envio": "3.0.0-alpha.18",
    "viem": "^2.0.0"
  },
  "optionalDependencies": {
    "generated": "./polymarket-indexer/../generated"
  },
  "engines": {
    "node": ">=22.0.0"
  }
}
```

## File: schema.graphql
```graphql
## ============================================================
## Phase 1A: Fee Module
## ============================================================

type FeeRefunded @index(fields: ["refundee", ["timestamp", "DESC"]]) {
  id: ID!
  orderHash: String! @index
  tokenId: String! @index
  timestamp: BigInt! @index
  refundee: String! @index
  feeRefunded: BigInt!
  feeCharged: BigInt!
  negRisk: Boolean!
}

## ============================================================
## Phase 1B: Sports Oracle
## ============================================================

type Game {
  id: ID!
  ancillaryData: String!
  ordering: String!
  state: String! @index
  homeScore: BigInt!
  awayScore: BigInt!
}

type Market {
  id: ID!
  gameId: String! @index
  state: String! @index
  marketType: String!
  underdog: String!
  line: BigInt!
  payouts: [BigInt!]!
}

## ============================================================
## Phase 2A: Wallet
## ============================================================

type Wallet {
  id: ID!
  signer: String! @index
  type: String!
  balance: BigInt!
  lastTransfer: BigInt!
  createdAt: BigInt!
}

type GlobalUSDCBalance {
  id: ID!
  balance: BigInt!
}

## ============================================================
## Phase 2B: Orderbook
## ============================================================

type MarketData {
  id: ID!
  condition: String! @index
  outcomeIndex: BigInt
  marketName: String!
  marketSlug: String! @index
  outcomes: String!
  description: String!
  image: String!
  startDate: String!
  endDate: String!
}

type OrderFilledEvent
  @index(fields: ["maker", ["timestamp", "DESC"]])
  @index(fields: ["makerAssetId", ["timestamp", "DESC"]]) {
  id: ID!
  transactionHash: String!
  timestamp: BigInt! @index
  orderHash: String! @index
  maker: String! @index
  taker: String! @index
  makerAssetId: String! @index
  takerAssetId: String! @index
  makerAmountFilled: BigInt!
  takerAmountFilled: BigInt!
  fee: BigInt!
}

type OrdersMatchedEvent {
  id: ID!
  timestamp: BigInt! @index
  makerAssetID: BigInt!
  takerAssetID: BigInt!
  makerAmountFilled: BigInt!
  takerAmountFilled: BigInt!
}

type Orderbook {
  id: ID!
  tradesQuantity: BigInt!
  buysQuantity: BigInt!
  sellsQuantity: BigInt!
  collateralVolume: BigInt!
  scaledCollateralVolume: BigDecimal!
  collateralBuyVolume: BigInt!
  scaledCollateralBuyVolume: BigDecimal!
  collateralSellVolume: BigInt!
  scaledCollateralSellVolume: BigDecimal!
}

type OrdersMatchedGlobal {
  id: ID!
  tradesQuantity: BigInt!
  buysQuantity: BigInt!
  sellsQuantity: BigInt!
  collateralVolume: BigInt!
  scaledCollateralVolume: BigDecimal!
  collateralBuyVolume: BigInt!
  scaledCollateralBuyVolume: BigDecimal!
  collateralSellVolume: BigInt!
  scaledCollateralSellVolume: BigDecimal!
}

## ============================================================
## Phase 3A: Open Interest
## ============================================================

type Condition {
  id: ID!
  positionIds: [BigInt!]!
  payoutNumerators: [BigInt!]!
  payoutDenominator: BigInt!
}

type NegRiskEvent {
  id: ID!
  feeBps: BigInt!
  questionCount: BigInt!
}

type MarketOpenInterest {
  id: ID!
  amount: BigInt!
}

type GlobalOpenInterest {
  id: ID!
  amount: BigInt!
}

## ============================================================
## Phase 3B: Activity
## ============================================================

type Split
  @index(fields: ["stakeholder", ["timestamp", "DESC"]])
  @index(fields: ["condition", ["timestamp", "DESC"]]) {
  id: ID!
  timestamp: BigInt! @index
  stakeholder: String! @index
  condition: String! @index
  amount: BigInt!
}

type Merge
  @index(fields: ["stakeholder", ["timestamp", "DESC"]])
  @index(fields: ["condition", ["timestamp", "DESC"]]) {
  id: ID!
  timestamp: BigInt! @index
  stakeholder: String! @index
  condition: String! @index
  amount: BigInt!
}

type Redemption
  @index(fields: ["redeemer", ["timestamp", "DESC"]])
  @index(fields: ["condition", ["timestamp", "DESC"]]) {
  id: ID!
  timestamp: BigInt! @index
  redeemer: String! @index
  condition: String! @index
  indexSets: [BigInt!]!
  payout: BigInt!
}

type NegRiskConversion
  @index(fields: ["stakeholder", ["timestamp", "DESC"]])
  @index(fields: ["negRiskMarketId", ["timestamp", "DESC"]]) {
  id: ID!
  timestamp: BigInt! @index
  stakeholder: String! @index
  negRiskMarketId: String! @index
  amount: BigInt!
  indexSet: BigInt!
  questionCount: BigInt!
}

type Position {
  id: ID!
  condition: String! @index
  outcomeIndex: BigInt!
}

type FixedProductMarketMaker {
  id: ID!
  creator: String! @index
  creationTimestamp: BigInt!
  creationTransactionHash: String!
  collateralToken: String!
  conditionalTokenAddress: String!
  conditions: [String!]!
  fee: BigInt!
  tradesQuantity: BigInt!
  buysQuantity: BigInt!
  sellsQuantity: BigInt!
  liquidityAddQuantity: BigInt!
  liquidityRemoveQuantity: BigInt!
  collateralVolume: BigInt!
  scaledCollateralVolume: BigDecimal!
  collateralBuyVolume: BigInt!
  scaledCollateralBuyVolume: BigDecimal!
  collateralSellVolume: BigInt!
  scaledCollateralSellVolume: BigDecimal!
  feeVolume: BigInt!
  scaledFeeVolume: BigDecimal!
  liquidityParameter: BigInt!
  scaledLiquidityParameter: BigDecimal!
  outcomeTokenAmounts: [BigInt!]!
  outcomeTokenPrices: [BigDecimal!]!
  outcomeSlotCount: BigInt
  lastActiveDay: BigInt!
  totalSupply: BigInt!
}

## ============================================================
## Phase 4A: PnL
## ============================================================

type UserPosition @index(fields: ["user", "tokenId"]) {
  id: ID!
  user: String! @index
  tokenId: BigInt! @index
  amount: BigInt!
  avgPrice: BigInt!
  realizedPnl: BigInt!
  totalBought: BigInt!
}

## ============================================================
## Phase 4B: FPMM
## ============================================================

type Collateral {
  id: ID!
  name: String!
  symbol: String!
  decimals: BigInt!
}

enum TransactionType {
  Buy
  Sell
}

type FpmmTransaction
  @index(fields: ["market_id", ["timestamp", "DESC"]])
  @index(fields: ["user", ["timestamp", "DESC"]]) {
  id: ID!
  type: TransactionType!
  timestamp: BigInt! @index
  market_id: String! @index
  user: String! @index
  tradeAmount: BigInt!
  feeAmount: BigInt!
  outcomeIndex: BigInt!
  outcomeTokensAmount: BigInt!
}

type FpmmFundingAddition
  @index(fields: ["fpmm_id", ["timestamp", "DESC"]])
  @index(fields: ["funder", ["timestamp", "DESC"]]) {
  id: ID!
  timestamp: BigInt! @index
  fpmm_id: String! @index
  funder: String! @index
  amountsAdded: [BigInt!]!
  amountsRefunded: [BigInt!]!
  sharesMinted: BigInt!
}

type FpmmFundingRemoval
  @index(fields: ["fpmm_id", ["timestamp", "DESC"]])
  @index(fields: ["funder", ["timestamp", "DESC"]]) {
  id: ID!
  timestamp: BigInt! @index
  fpmm_id: String! @index
  funder: String! @index
  amountsRemoved: [BigInt!]!
  collateralRemoved: BigInt!
  sharesBurnt: BigInt!
}

type FpmmPoolMembership {
  id: ID!
  pool_id: String! @index
  funder: String! @index
  amount: BigInt!
}
```
