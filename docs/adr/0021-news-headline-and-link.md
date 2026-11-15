# ADR-0021: News as headline and link

Status: Accepted
Date: 2026-11-15

## Context

- El Espectador's terms forbid reproduction and commercial use without written authorisation, and reproducing article text is an anti-goal (section 2).
- The only live feed is `https://www.elespectador.com/arc/outboundfeeds/discover/category/economia/`. No licensed news feed exists, and El Espectador may refuse permission for more (risk 1).
- The news limit for Lite is a **Call** of the designer, not confirmed by the owner. The headline and link rule needs no permission and is firm.

## Decision

- Show headline and link only, from the El Espectador feed.
- Store title, link, date and section, never the body: `news_articles` has no body column.
- Lite sees the 8 latest headlines; Pro sees all, paginated.
- Headlines are matched to instruments by alias with each news run.

## Consequences

- Cost: the app cannot show article text, and users leave Barras to read a story.
- The schema has no place to store a body, so the rule holds by construction. The ingestion contract test uses recorded responses (ADR-0012).
- The limit follows the plan entitlements (ADR-0010).
