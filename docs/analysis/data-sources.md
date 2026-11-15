# Data Sources for Colombia

Research done on 2026-11-15 for the foundation design. Each source sits behind a port, so this document records access and limits, not code.

Legend: **[V]** checked on a live page or endpoint that day. **[S]** read in a secondary source that day. **[U]** could not be verified.

## Summary

| Data | Source | Access | Freshness | Cost | Main caveat |
| --- | --- | --- | --- | --- | --- |
| News | El Espectador | RSS (Discover feed) | near real time | free | headline and link only |
| CPI, unemployment | DANE | XLSX per release | monthly | free | no API; file layout can change |
| USD/COP (TRM) | datos.gov.co | Socrata REST | daily | free | none |
| Other currency pairs | open.er-api.com | REST | daily | free | attribution link; no redistribution |
| Colombian stocks | nuam (formerly BVC), or a vendor | licence or vendor API | end of day | paid | real time needs an exchange licence |
| Crypto | Binance, Buda | REST and websocket | real time | free | display terms unverified |
| Bank product rates | datos.gov.co | Socrata REST | daily | free | none |
| Bank accounts | none usable | none | none | none | no open-finance API before about 2028 |

## 1. El Espectador

- **Access:** RSS. The site now runs on WordPress. The old `/arc/outboundfeeds/rss/...` path returns 404. `/feed/` and `/economia/feed/` answer but are empty. The only live feed is the Google Discover one: 20 items with full body, newest item the same day. [V]
- **URL:** `https://www.elespectador.com/arc/outboundfeeds/discover/category/economia/`
- **Freshness:** near real time. **Cost:** free, no key.
- **Terms:** linking without permission is allowed. Reproduction or commercial use needs written authorisation. RSS is not mentioned. [V]
- **Upshot:** show the headline and the link. Excerpts need a licence. Barras never stores or serves the body, even though the feed carries it.
- **Confidence:** high on the feeds, medium on the legal reading.
- **Source:** https://www.elespectador.com/terminos/terminos-y-condiciones/

## 2. DANE

- **Access:** no API for headline indicators. One XLSX and PDF per release. datos.gov.co holds only DIVIPOLA and microdata links from DANE. [V]
- **URLs (both answered 200):** [V]
  - `https://www.dane.gov.co/files/operaciones/IPC/oct2026/anex-IPC-oct2026.xlsx`
  - `https://www.dane.gov.co/files/operaciones/GEIH/anex-GEIH-sep2026.xlsx`
- **GDP file pattern:** [U]
- **Fallback with an API:** OECD SDMX (`sdmx.oecd.org`) returns Colombia's monthly CPI and unemployment as CSV, through 2026-09. It lags about a month. [V]
- **Freshness:** CPI and labour monthly, GDP quarterly. **Cost:** free.
- **Terms:** reuse is allowed with the citation "Fuente: DANE". There is no Creative Commons licence, and commercial use by third parties is not addressed. [V]
- **Confidence:** medium.
- **Source:** https://www.dane.gov.co/index.php/terrminos-y-condiciones-dane

## 3. TRM and other currency rates

- **Access:** Socrata REST, no key.
- **Datasets (Superintendencia Financiera):** `32sa-8pi3` (TRM) and `mcec-87by` (history). Example: `https://www.datos.gov.co/resource/32sa-8pi3.json` [V]
- **Alternative:** Banco de la República SDMX at `https://totoro.banrep.gov.co/nsi-jax-ws/rest/` (`DF_TRM_DAILY_LATEST`; also COLCAP monthly). [V]
- **Freshness:** daily. On 13 Nov at 23:05 UTC it carried the rate valid 14 to 17 Nov. [V]
- **Cost:** free. Licence CC BY-SA 4.0. [V]
- **Other pairs:** `open.er-api.com/v6/latest/COP` is daily and free, allows commercial use, requires an attribution link and forbids redistribution. [V] The ECB feed has no COP. [V] EUR/COP can be derived from TRM times the ECB's EUR/USD and must be labelled indicative.
- **Confidence:** high.

## 4. Colombian stocks

- **Official:** nuam (formerly BVC) licenses real-time, delayed (at least 15 minutes before redistribution) and end-of-day data. Contact `publicaciones@bvc.com.co`. There is no public price list. [V] Fees are an annual fixed amount plus a fee per data feed. [S, from 2022]
- **Twelve Data:** exchange code XBOG, 57 symbols, end of day only. It needs the Ultra personal plan or the Enterprise business plan, listed at USD 1,099 per month, which includes external display. The page showed conflicting prices. [V]
- **EODHD:** Colombia is absent. [V]
- **Finnhub:** lists XBOG. [V] Outside the United States it is end of day only, and plans are for personal use. [S]
- **Polygon (now Massive):** United States only. [U]
- **Yahoo Finance:** `ECOPETROL.CL` resolves with a 20-minute delay. [V] There is no official API or redistribution right. [U] Barras does not use it.
- **Confidence:** medium.
- **Sources:** https://www.bvc.com.co/informacion-y-medios-electronicos, https://twelvedata.com/pricing-business

## 5. Crypto

| Source | REST | Websocket | Limits | COP pairs | Display terms |
| --- | --- | --- | --- | --- | --- |
| Binance | `https://data-api.binance.vision`, no key | `wss://data-stream.binance.vision`, free | 6,000 weight per minute per IP; 1,024 streams per connection [V] | only USDTCOP [V] | [U] |
| Bitso | public | `wss://ws.bitso.com` | 60 requests per minute [V] | `usd_cop`, `usdt_cop` [V] | not checked |
| Buda | `https://www.buda.com/api/v2` | `wss://realtime.buda.com` | 120 requests per minute [V] | BTC, ETH, SOL, USDC, USDT, LTC, BCH [V] | [U] |
| CoinGecko | Demo plan: 10,000 calls per month | from USD 129 per month | as per plan [V] | n/a | "Powered by CoinGecko" attribution; no API resale [V] |
| CoinMarketCap | free: 15,000 credits per month, 50 requests per minute | from USD 79 per month, still "under development" | as per plan [V] | n/a | commercial use for one product up to 100,000 users [V] |

- **Confidence:** high on the limits, low on Binance and Buda display rights.

## 6. Banks

- **Regulation:** Decreto 368 (7 April 2026) makes open finance mandatory for entities supervised by the Superintendencia Financiera. Decreto 1489 moved the standards schedule to 7 April 2027 and the participant directory to 7 October 2027. Entities then get 12 months per standard. [S] Apps that are not supervised join only through voluntary schemes. No regulated API will be usable before about 2028.
- **Aggregators:** Belvo's documentation lists banking aggregation for Brazil only; its coverage of Bancolombia, Davivienda and Nu is [U]. Prometeo's documentation is behind a password; its site markets verification and payments; coverage [U]. Both price through sales.
- **Open data on rates:** dataset `axk9-g2nh` has CDT, CDAT and savings rates by entity, daily, CC BY-SA. [V] Lending rates are in `qzsc-9esp`.
- **Confidence:** medium on regulation, low on aggregators.
- **Source:** https://incp.org.co/?p=136896

## 7. Regulation of suggestions

This is the practical upshot, not legal advice.

- Under Decreto 661 of 2018, "asesoría" is reserved to supervised entities. It is triggered by a "recomendación profesional": one made for an identified investor using their profile (articles 2.40.1.1.1 and 2.40.1.1.2). [V]
- General communications and research reports are excluded and need no licence. They must carry a disclaimer and must never imply they suit the reader (article 2.40.1.1.3). [V]
- A generic trend list marked "informativo, no constituye asesoría" is defensible. Suggestions personalised from the user's tracked finances risk being regulated advice.
- Comparable apps partner with licensed firms. tyba is a "corresponsal digital" of Credicorp Capital and says it cannot provide financial services itself. [V]

## Biggest risks for the product

1. There is no licensable news feed, and El Espectador forbids commercial reuse.
2. Intraday prices for Colombian stocks need an exchange licence of unknown cost; vendors give end of day only.
3. There are no open-finance bank APIs until about 2028, and aggregator coverage is unverified. Plan for manual entry.
4. DANE has no API, and file-download adapters are brittle.
5. Personalised suggestions may be regulated advice. Get a lawyer's review.
