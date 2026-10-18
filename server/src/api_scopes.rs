//! Port of `lib/api-scopes.ts`: the API-token scope lattice. Pure data and
//! pure functions, no database.
//!
//! Four scopes, because each one answers a question somebody actually asks:
//!
//!   upload  Put files in. Cannot see, change or delete anything.   (ShareX)
//!   read    See files, folders and links. Cannot change anything.  (a viewer)
//!   write   Everything above, plus changing and deleting.          (the app)
//!   full    Everything, including administrator endpoints.
//!
//! They are deliberately NOT a rank. `upload` and `read` are incomparable, so
//! every comparison below is a SUBSET test over the capability sets, never
//! `a >= b`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenScope {
    Upload,
    Read,
    Write,
    Full,
}

pub const TOKEN_SCOPES: [TokenScope; 4] = [TokenScope::Upload, TokenScope::Read, TokenScope::Write, TokenScope::Full];

impl TokenScope {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenScope::Upload => "upload",
            TokenScope::Read => "read",
            TokenScope::Write => "write",
            TokenScope::Full => "full",
        }
    }
}

impl std::fmt::Display for TokenScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a call needs, as opposed to what a token has. Every surface that
/// accepts a token names one; `identify` is granted by every scope and asked
/// for by exactly one surface (`GET /api/me`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenCapability {
    Identify,
    Upload,
    Read,
    Write,
    Admin,
}

pub const TOKEN_CAPABILITIES: [TokenCapability; 5] = [
    TokenCapability::Identify,
    TokenCapability::Upload,
    TokenCapability::Read,
    TokenCapability::Write,
    TokenCapability::Admin,
];

use TokenCapability as C;

/// `GRANTS`
pub fn capabilities_of(scope: TokenScope) -> &'static [TokenCapability] {
    match scope {
        TokenScope::Upload => &[C::Identify, C::Upload],
        TokenScope::Read => &[C::Identify, C::Read],
        TokenScope::Write => &[C::Identify, C::Upload, C::Read, C::Write],
        TokenScope::Full => &[C::Identify, C::Upload, C::Read, C::Write, C::Admin],
    }
}

/// `SCOPE_FOR`: the narrowest scope that grants a capability - what a 403
/// tells the caller to go and create.
pub fn scope_for(capability: TokenCapability) -> TokenScope {
    match capability {
        // Every scope grants it, so `upload` is simply the narrowest of them.
        C::Identify => TokenScope::Upload,
        C::Upload => TokenScope::Upload,
        C::Read => TokenScope::Read,
        C::Write => TokenScope::Write,
        C::Admin => TokenScope::Full,
    }
}

/// The scope legacy rows are backfilled to (drizzle/0012_api_token_scopes.sql).
pub const LEGACY_TOKEN_SCOPE: TokenScope = TokenScope::Full;

/// `isTokenScope(value)`: the four exact strings and nothing else.
pub fn parse_token_scope(value: &str) -> Option<TokenScope> {
    TOKEN_SCOPES.into_iter().find(|s| s.as_str() == value)
}

pub fn is_token_scope(value: &str) -> bool {
    parse_token_scope(value).is_some()
}

pub fn scope_grants(scope: TokenScope, capability: TokenCapability) -> bool {
    capabilities_of(scope).contains(&capability)
}

/// May a token holding `current` be changed to `next` without creating a new
/// one? Only when `next` is a PROPER SUBSET of `current`; equal is refused too.
pub fn is_narrowing(current: TokenScope, next: TokenScope) -> bool {
    if current == next {
        return false;
    }
    capabilities_of(next).iter().all(|cap| capabilities_of(current).contains(cap))
}

/// Every scope a token at `current` may be narrowed to, for the UI to offer.
pub fn narrower_scopes(current: TokenScope) -> Vec<TokenScope> {
    TOKEN_SCOPES.into_iter().filter(|s| is_narrowing(current, *s)).collect()
}

#[cfg(test)]
mod tests {
    //! tests/unit/api-scopes.test.ts
    use super::*;
    use TokenScope::*;

    #[test]
    fn upload_only_tokens_have_exactly_one_power_beyond_identifying_themselves() {
        assert_eq!(capabilities_of(Upload), &[C::Identify, C::Upload]);
        assert!(scope_grants(Upload, C::Upload));
        assert!(!scope_grants(Upload, C::Read));
        assert!(!scope_grants(Upload, C::Write));
        assert!(!scope_grants(Upload, C::Admin));
    }

    #[test]
    fn a_read_token_sees_and_nothing_else() {
        assert!(scope_grants(Read, C::Read));
        assert!(!scope_grants(Read, C::Upload));
        assert!(!scope_grants(Read, C::Write));
    }

    #[test]
    fn a_write_token_has_everything_except_administrator_endpoints() {
        assert!(scope_grants(Write, C::Upload));
        assert!(scope_grants(Write, C::Read));
        assert!(scope_grants(Write, C::Write));
        assert!(!scope_grants(Write, C::Admin));
    }

    #[test]
    fn full_has_everything() {
        for cap in TOKEN_CAPABILITIES {
            assert!(scope_grants(Full, cap));
        }
    }

    #[test]
    fn every_scope_may_identify_itself() {
        for scope in TOKEN_SCOPES {
            assert!(scope_grants(scope, C::Identify));
        }
    }

    #[test]
    fn names_for_every_capability_the_narrowest_scope_that_grants_it() {
        for cap in TOKEN_CAPABILITIES {
            let scope = scope_for(cap);
            assert!(scope_grants(scope, cap));
            for other in narrower_scopes(scope) {
                assert!(!scope_grants(other, cap));
            }
        }
    }

    #[test]
    fn allows_only_a_proper_subset() {
        assert!(is_narrowing(Full, Write));
        assert!(is_narrowing(Full, Read));
        assert!(is_narrowing(Full, Upload));
        assert!(is_narrowing(Write, Read));
        assert!(is_narrowing(Write, Upload));
    }

    #[test]
    fn refuses_every_widening() {
        assert!(!is_narrowing(Read, Write));
        assert!(!is_narrowing(Read, Full));
        assert!(!is_narrowing(Upload, Write));
        assert!(!is_narrowing(Write, Full));
    }

    #[test]
    fn refuses_a_no_op() {
        for scope in TOKEN_SCOPES {
            assert!(!is_narrowing(scope, scope));
        }
    }

    #[test]
    fn upload_and_read_are_incomparable() {
        assert!(!is_narrowing(Upload, Read));
        assert!(!is_narrowing(Read, Upload));
    }

    #[test]
    fn offers_only_legal_choices() {
        let mut full = narrower_scopes(Full);
        full.sort_by_key(|s| s.as_str());
        assert_eq!(full, vec![Read, Upload, Write]);
        let mut write = narrower_scopes(Write);
        write.sort_by_key(|s| s.as_str());
        assert_eq!(write, vec![Read, Upload]);
        assert!(narrower_scopes(Upload).is_empty());
        assert!(narrower_scopes(Read).is_empty());
    }

    #[test]
    fn agrees_with_the_capability_sets_it_compares() {
        for from in TOKEN_SCOPES {
            for to in TOKEN_SCOPES {
                if !is_narrowing(from, to) {
                    continue;
                }
                for cap in capabilities_of(to) {
                    assert!(scope_grants(from, *cap));
                }
                assert!(capabilities_of(to).len() < capabilities_of(from).len());
            }
        }
    }

    #[test]
    fn accepts_the_four_scopes_and_nothing_else() {
        for scope in TOKEN_SCOPES {
            assert!(is_token_scope(scope.as_str()));
        }
        for junk in ["", "FULL", "admin", "readwrite", "3"] {
            assert!(!is_token_scope(junk));
        }
    }

    #[test]
    fn backfills_legacy_rows_to_full() {
        assert_eq!(LEGACY_TOKEN_SCOPE, Full);
    }
}
