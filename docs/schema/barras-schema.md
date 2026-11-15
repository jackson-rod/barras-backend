# Barras schema

The initial schema, in the Postgres schema `barras`. It is a starting point, built from the owner's table list. Each module's spec refines its tables.

## Conventions

- Ids are UUID v7.
- Timestamps are `timestamptz` in UTC.
- Fiat amounts are `numeric(20,2)` and quantities are `numeric(38,18)`, always with a currency.

## identity

### users

The people who sign up. Columns are defined in this module's spec.

### user_identities

How a user signs in: password, Apple or Google. Columns are defined in this module's spec.

### sessions

Sessions, with hashed refresh tokens. Columns are defined in this module's spec.

### email_challenges

Verification codes sent by email. Columns are defined in this module's spec.

### roles

The roles a user can hold. Columns are defined in this module's spec.

### permissions

The permission catalog, seeded from code. Columns are defined in this module's spec.

### role_permissions

The permissions granted to each role. Columns are defined in this module's spec.

## settings

### user_settings

Display currency, language, time zone, notification preferences and whether onboarding is completed. Columns are defined in this module's spec.

### currencies

The currency list. Columns are defined in this module's spec.

### device_tokens

Device tokens for push notifications. Columns are defined in this module's spec.

## billing

### plans

The plans, Lite and Pro. Columns are defined in this module's spec.

### subscriptions

Each user's subscription state. Columns are defined in this module's spec.

### subscription_events

Store events, handled idempotently. Columns are defined in this module's spec.

## capital

### banks

The catalog of banks. Columns are defined in this module's spec.

### exchanges

The catalog of exchanges. Columns are defined in this module's spec.

### accounts

Where money is held. Columns are defined in this module's spec.

### categories

Categories for income, expenses and budgets. Columns are defined in this module's spec.

### transactions

Income and expense records. Columns are defined in this module's spec.

### investments

The user's investments. Columns are defined in this module's spec.

### investment_movements

The movements of each investment. Columns are defined in this module's spec.

### capital_snapshots

One row per user per day, with the total and per class, written by a daily job. Columns are defined in this module's spec.

### receipts

Scanned receipts that become expense drafts. Columns are defined in this module's spec.

## budgets

### budgets

Budgets per category and period; spending is computed from `transactions`. Columns are defined in this module's spec.

## goals

### goals

The user's savings goals. Columns are defined in this module's spec.

### goal_contributions

Contributions to goals, each linked to the account that holds the money. Columns are defined in this module's spec.

## markets

### instruments

The curated catalog of instruments. Columns are defined in this module's spec.

### instrument_aliases

Aliases used to match news headlines to instruments. Columns are defined in this module's spec.

### market_bars

Price bars, partitioned by month. Columns are defined in this module's spec.

### quotes_latest

The latest quote of each instrument. Columns are defined in this module's spec.

### trends

Current trend scores. Columns are defined in this module's spec.

### trend_history

Trend history. Columns are defined in this module's spec.

### favorites

The instruments a user favourites. Columns are defined in this module's spec.

### watchlists

User-made watchlists. Columns are defined in this module's spec.

### watchlist_items

The instruments in each watchlist. Columns are defined in this module's spec.

## news

### news_sources

The news sources. Columns are defined in this module's spec.

### news_articles

Headlines with link, date and section; no body. Columns are defined in this module's spec.

### news_article_instruments

Links between news articles and instruments. Columns are defined in this module's spec.

## indicators

### indicator_series

Official figure series. Columns are defined in this module's spec.

### indicator_observations

Observations of each series. Columns are defined in this module's spec.

### fx_rates

Currency rates. Columns are defined in this module's spec.

### bank_rates

Published CDT and savings rates per bank. Columns are defined in this module's spec.

## assistant

### assistant_conversations

Chatbot conversations. Columns are defined in this module's spec.

### assistant_messages

The messages of each conversation. Columns are defined in this module's spec.

## operations

### ingest_runs

A record of each ingestion job run. Columns are defined by the ingest foundation ticket (F-06, BAR-6).

## Seeds

The role `trader` and the permission catalog; the plans; the currency list; Bancolombia, Davivienda and Nu; Binance, Buda and the Colombian stock exchange; a demo user for local development.
