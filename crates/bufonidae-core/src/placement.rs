//! Placement policies: which account stores a given file.

/// Identifier of a connected account inside a pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountId(pub u32);

/// Storage state of an account, as seen by the planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountSpace {
    pub id: AccountId,
    pub free_bytes: u64,
}

/// Chooses the account that should store a file of `size` bytes.
pub trait Placement {
    /// Returns `None` when no account can hold the file.
    fn choose(&self, accounts: &[AccountSpace], size: u64) -> Option<AccountId>;
}

/// Picks the account with the most free space, keeping `headroom_bytes` free
/// on it after the file is stored.
#[derive(Debug, Clone, Copy, Default)]
pub struct MostFreeSpace {
    pub headroom_bytes: u64,
}

impl Placement for MostFreeSpace {
    fn choose(&self, accounts: &[AccountSpace], size: u64) -> Option<AccountId> {
        let needed = size.checked_add(self.headroom_bytes)?;
        accounts
            .iter()
            .filter(|a| a.free_bytes >= needed)
            .max_by_key(|a| a.free_bytes)
            .map(|a| a.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(id: u32, free_bytes: u64) -> AccountSpace {
        AccountSpace {
            id: AccountId(id),
            free_bytes,
        }
    }

    #[test]
    fn picks_account_with_most_free_space() {
        let accounts = [acc(1, 10), acc(2, 50), acc(3, 30)];
        let policy = MostFreeSpace::default();
        assert_eq!(policy.choose(&accounts, 5), Some(AccountId(2)));
    }

    #[test]
    fn skips_accounts_that_cannot_hold_the_file_plus_headroom() {
        let accounts = [acc(1, 100), acc(2, 40)];
        let policy = MostFreeSpace { headroom_bytes: 20 };
        assert_eq!(policy.choose(&accounts, 30), Some(AccountId(1)));
        assert_eq!(policy.choose(&accounts, 90), None);
    }

    #[test]
    fn size_overflow_yields_none() {
        let accounts = [acc(1, u64::MAX)];
        let policy = MostFreeSpace { headroom_bytes: 1 };
        assert_eq!(policy.choose(&accounts, u64::MAX), None);
    }
}
