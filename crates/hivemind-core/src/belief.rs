//! Belief labels. The values are computed at read time by a named, versioned
//! policy from evidence, edges, and feedback (`docs/03-memory-model.md`). They
//! are never written by clients.

/// Name of the policy that derives [`Verification`] and [`TrustTier`]. Changing
/// the rules means a new name, so results stay explainable.
pub const BELIEF_POLICY: &str = "belief-v0";

str_enum! {
    /// Lifecycle status, set only by lifecycle events.
    pub enum Status {
        /// Current belief.
        Active => "active",
        /// The world changed: a newer claim replaced this one. It was true for its period.
        Superseded => "superseded",
        /// The world changed: this stopped being true, with no replacement.
        Invalidated => "invalidated",
        /// Knowledge changed: this was wrong, or was recorded wrongly.
        Retracted => "retracted",
    }
}

str_enum! {
    pub enum Verification {
        /// One evidence group.
        Unverified => "unverified",
        /// Two or more independent evidence groups.
        Corroborated => "corroborated",
        /// Confirmed by a human, or by a reproducible check referenced as evidence.
        Verified => "verified",
        /// An active claim contradicts this one.
        Disputed => "disputed",
    }
}

str_enum! {
    /// How far a claim may be relied on. Ordered from least to most trusted.
    pub enum TrustTier {
        /// Derived only from external content (web pages, issue text, third-party output).
        Untrusted => "untrusted",
        /// Asserted by an agent.
        Agent => "agent",
        /// Independently corroborated.
        Corroborated => "corroborated",
        /// Reviewed by a human.
        Human => "human",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_tiers_are_ordered() {
        assert!(TrustTier::Untrusted < TrustTier::Agent);
        assert!(TrustTier::Agent < TrustTier::Corroborated);
        assert!(TrustTier::Corroborated < TrustTier::Human);
    }

    #[test]
    fn parses_kebab_and_snake_case() {
        assert_eq!("Active".parse::<Status>().unwrap(), Status::Active);
        assert!("pending".parse::<Status>().is_err());
    }
}
