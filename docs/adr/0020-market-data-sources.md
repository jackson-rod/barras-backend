# ADR-0020: Market data sources

Status: Accepted
Date: 2026-11-15

## Context

- The owner named the sources: exchanges and the stock market, DANE, and banks (Decided, section 1).
- The exchange (BVC, now nuam) licenses real-time data by contract only. The vendor found with display rights lists about USD 1,099 per month, and Yahoo has no redistribution right.
- The terms of Binance and Buda for showing data in a commercial app are unverified (risk 3).

## Decision

- Crypto: the Binance public websocket and Buda (COP pairs: BTC, ETH, SOL, USDC, USDT, LTC, BCH).
- Colombian stocks: end-of-day prices only until a licence is bought; the vendor is chosen in the licensing spike F-10. Yahoo is not used.
- TRM and bank rates: datos.gov.co, datasets `32sa-8pi3` and `axk9-g2nh`. Other currency pairs: `open.er-api.com`.
- DANE: monthly XLSX files cited as "Fuente: DANE", with OECD SDMX as the fallback.
- If F-10 ends without a licence, the first version ships with crypto, news and official figures. Stocks stay in the catalog, valued at the last price the user entered.

## Consequences

- Cost: stock prices are not live, and the product may ship without any stock prices.
- Each source is a port with attribution and a contract test (ADR-0012), and its job is in ADR-0013.
- The DANE adapter breaks when the file layout changes (risk 4).
