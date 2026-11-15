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

/// Places a batch of files, largest first (first-fit-decreasing), tracking the
/// space each placement consumes. Returns, for each entry of `sizes` and in
/// the same order, the chosen account or `None` when nothing could hold it.
#[must_use]
pub fn plan_batch(
    policy: &impl Placement,
    accounts: &[AccountSpace],
    sizes: &[u64],
) -> Vec<Option<AccountId>> {
    let mut remaining = accounts.to_vec();
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sizes[i]));

    let mut plan = vec![None; sizes.len()];
    for i in order {
        if let Some(id) = policy.choose(&remaining, sizes[i]) {
            if let Some(a) = remaining.iter_mut().find(|a| a.id == id) {
                a.free_bytes = a.free_bytes.saturating_sub(sizes[i]);
            }
            plan[i] = Some(id);
        }
    }
    plan
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

    #[test]
    fn batch_spreads_files_and_reports_unplaced() {
        let accounts = [acc(1, 100), acc(2, 100)];
        let plan = plan_batch(&MostFreeSpace::default(), &accounts, &[60, 60, 50, 500]);
        assert_eq!(plan[3], None);
        // After the two 60s each account has 40 left: the 50 fits nowhere.
        assert_eq!(plan[2], None);
        assert!(plan[0].is_some() && plan[1].is_some());
        assert_ne!(plan[0], plan[1]);
    }

    #[test]
    fn batch_never_overcommits_an_account() {
        let accounts = [acc(1, 100), acc(2, 30)];
        let sizes = [40, 40, 20, 10];
        let plan = plan_batch(&MostFreeSpace::default(), &accounts, &sizes);
        for a in &accounts {
            let used: u64 = plan
                .iter()
                .zip(sizes)
                .filter(|(p, _)| **p == Some(a.id))
                .map(|(_, s)| s)
                .sum();
            assert!(used <= a.free_bytes);
        }
        assert!(plan.iter().all(Option::is_some));
    }
}
