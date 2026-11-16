//! What a user's plan lets them use.

use time::OffsetDateTime;

/// A plan a user can be on. A new user starts on `Lite`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Plan {
    Lite,
    Pro,
}

impl Plan {
    /// The stable code of the plan. It is public API, like an error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Lite => "LITE",
            Self::Pro => "PRO",
        }
    }

    /// The catalog: what each plan unlocks beyond what every user has.
    pub const fn features(self) -> &'static [Feature] {
        match self {
            Self::Lite => &[],
            Self::Pro => Feature::ALL,
        }
    }
}

/// Something only a plan unlocks. A route names the feature it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    Assistant,
    AllSuggestions,
    AllNews,
    FullHistory,
    StockTracker,
}

impl Feature {
    pub const ALL: &'static [Self] = &[
        Self::Assistant,
        Self::AllSuggestions,
        Self::AllNews,
        Self::FullHistory,
        Self::StockTracker,
    ];

    /// The stable code of the feature. It is public API, like an error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Assistant => "ASSISTANT",
            Self::AllSuggestions => "ALL_SUGGESTIONS",
            Self::AllNews => "ALL_NEWS",
            Self::FullHistory => "FULL_HISTORY",
            Self::StockTracker => "STOCK_TRACKER",
        }
    }
}

/// A plan and, optionally, the instant it ends. From that instant on the plan counts as
/// `Lite`, so a trial ends on its own, with no event from a store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entitlements {
    plan: Plan,
    expires_at: Option<OffsetDateTime>,
}

impl Entitlements {
    pub const fn new(plan: Plan, expires_at: Option<OffsetDateTime>) -> Self {
        Self { plan, expires_at }
    }

    /// What a new user has.
    pub const fn lite() -> Self {
        Self::new(Plan::Lite, None)
    }

    pub fn effective_plan(&self, now: OffsetDateTime) -> Plan {
        match self.expires_at {
            Some(expires_at) if now >= expires_at => Plan::Lite,
            _ => self.plan,
        }
    }

    pub fn allows(&self, feature: Feature, now: OffsetDateTime) -> bool {
        self.effective_plan(now).features().contains(&feature)
    }
}

#[cfg(test)]
mod tests {
    use time::Duration;

    use super::*;

    fn expiry() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap()
    }

    fn allowed(entitlements: Entitlements, now: OffsetDateTime) -> Vec<Feature> {
        Feature::ALL
            .iter()
            .copied()
            .filter(|feature| entitlements.allows(*feature, now))
            .collect()
    }

    #[test]
    fn plans_and_features_keep_their_codes() {
        assert_eq!(Plan::Lite.code(), "LITE");
        assert_eq!(Plan::Pro.code(), "PRO");
        let codes: Vec<&str> = Feature::ALL.iter().map(|feature| feature.code()).collect();
        assert_eq!(
            codes,
            [
                "ASSISTANT",
                "ALL_SUGGESTIONS",
                "ALL_NEWS",
                "FULL_HISTORY",
                "STOCK_TRACKER"
            ]
        );
    }

    #[test]
    fn all_lists_each_of_the_five_features_once() {
        assert_eq!(
            Feature::ALL,
            [
                Feature::Assistant,
                Feature::AllSuggestions,
                Feature::AllNews,
                Feature::FullHistory,
                Feature::StockTracker,
            ]
        );
    }

    #[test]
    fn the_catalog_gives_pro_every_feature_and_lite_none() {
        assert_eq!(Plan::Pro.features(), Feature::ALL);
        assert!(Plan::Lite.features().is_empty());
    }

    #[test]
    fn pro_with_no_expiry_allows_every_feature() {
        let pro = Entitlements::new(Plan::Pro, None);
        for now in [expiry(), expiry() + Duration::days(36_500)] {
            assert_eq!(pro.effective_plan(now), Plan::Pro);
            assert_eq!(allowed(pro, now), Feature::ALL);
        }
    }

    #[test]
    fn lite_allows_none_of_the_five() {
        for lite in [
            Entitlements::lite(),
            Entitlements::new(Plan::Lite, None),
            Entitlements::new(Plan::Lite, Some(expiry() + Duration::days(7))),
        ] {
            assert_eq!(lite.effective_plan(expiry()), Plan::Lite);
            assert!(allowed(lite, expiry()).is_empty());
        }
    }

    #[test]
    fn a_new_user_starts_on_lite_with_no_expiry() {
        assert_eq!(Entitlements::lite(), Entitlements::new(Plan::Lite, None));
    }

    #[test]
    fn pro_counts_as_lite_from_the_instant_it_expires() {
        let trial = Entitlements::new(Plan::Pro, Some(expiry()));

        let just_before = expiry() - Duration::nanoseconds(1);
        assert_eq!(trial.effective_plan(just_before), Plan::Pro);
        assert_eq!(allowed(trial, just_before), Feature::ALL);

        for now in [expiry(), expiry() + Duration::seconds(1)] {
            assert_eq!(trial.effective_plan(now), Plan::Lite);
            assert!(allowed(trial, now).is_empty());
        }
    }
}
