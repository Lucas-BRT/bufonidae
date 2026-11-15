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

/// A file to place, with the number of copies wanted on distinct accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    pub size: u64,
    pub copies: u8,
}

/// Picks one account for `size` bytes among those not in `exclude`.
#[must_use]
pub fn choose_distinct(
    policy: &impl Placement,
    accounts: &[AccountSpace],
    exclude: &[AccountId],
    size: u64,
) -> Option<AccountId> {
    let candidates: Vec<AccountSpace> = accounts
        .iter()
        .filter(|a| !exclude.contains(&a.id))
        .copied()
        .collect();
    policy.choose(&candidates, size)
}

/// Places a batch of files, largest first (first-fit-decreasing), tracking the
/// space each placement consumes. Each file gets all its copies on distinct
/// accounts, or nothing: `None` (and no space reserved) when it cannot get
/// every copy, so a file is never silently under-protected. Results are in
/// the order of `wanted`.
#[must_use]
pub fn plan_batch_copies(
    policy: &impl Placement,
    accounts: &[AccountSpace],
    wanted: &[Wanted],
) -> Vec<Option<Vec<AccountId>>> {
    let mut remaining = accounts.to_vec();
    let mut order: Vec<usize> = (0..wanted.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(wanted[i].size));

    let mut plan = vec![None; wanted.len()];
    for i in order {
        let Wanted { size, copies } = wanted[i];
        let mut trial = remaining.clone();
        let mut chosen = Vec::new();
        for _ in 0..copies.max(1) {
            let Some(id) = choose_distinct(policy, &trial, &chosen, size) else {
                break;
            };
            if let Some(a) = trial.iter_mut().find(|a| a.id == id) {
                a.free_bytes = a.free_bytes.saturating_sub(size);
            }
            chosen.push(id);
        }
        if chosen.len() == usize::from(copies.max(1)) {
            remaining = trial;
            plan[i] = Some(chosen);
        }
    }
    plan
}

/// Single-copy [`plan_batch_copies`].
#[must_use]
pub fn plan_batch(
    policy: &impl Placement,
    accounts: &[AccountSpace],
    sizes: &[u64],
) -> Vec<Option<AccountId>> {
    let wanted: Vec<Wanted> = sizes
        .iter()
        .map(|&size| Wanted { size, copies: 1 })
        .collect();
    plan_batch_copies(policy, accounts, &wanted)
        .into_iter()
        .map(|p| p.and_then(|ids| ids.first().copied()))
        .collect()
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

    #[test]
    fn copies_land_on_distinct_accounts() {
        let accounts = [acc(1, 100), acc(2, 100), acc(3, 100)];
        let plan = plan_batch_copies(
            &MostFreeSpace::default(),
            &accounts,
            &[
                Wanted {
                    size: 10,
                    copies: 2,
                },
                Wanted {
                    size: 10,
                    copies: 3,
                },
            ],
        );
        for ids in plan.iter().flatten() {
            let mut sorted = ids.clone();
            sorted.sort_by_key(|a| a.0);
            sorted.dedup();
            assert_eq!(sorted.len(), ids.len(), "no account twice for one file");
        }
        assert_eq!(plan[0].as_ref().unwrap().len(), 2);
        assert_eq!(plan[1].as_ref().unwrap().len(), 3);
    }

    #[test]
    fn missing_copies_mean_no_placement_and_no_reserved_space() {
        // Two accounts cannot hold three copies; the second file must still
        // see all the space the failed one did not take.
        let accounts = [acc(1, 100), acc(2, 100)];
        let plan = plan_batch_copies(
            &MostFreeSpace::default(),
            &accounts,
            &[
                Wanted {
                    size: 90,
                    copies: 3,
                },
                Wanted {
                    size: 90,
                    copies: 2,
                },
            ],
        );
        assert_eq!(plan[0], None);
        assert_eq!(plan[1].as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn mirror_does_not_overcommit_any_account() {
        let accounts = [acc(1, 100), acc(2, 60), acc(3, 60)];
        let wanted: Vec<Wanted> = [50, 40, 30, 30]
            .iter()
            .map(|&size| Wanted { size, copies: 2 })
            .collect();
        let plan = plan_batch_copies(&MostFreeSpace::default(), &accounts, &wanted);
        for a in &accounts {
            let used: u64 = plan
                .iter()
                .zip(&wanted)
                .filter(|(p, _)| p.as_ref().is_some_and(|ids| ids.contains(&a.id)))
                .map(|(_, w)| w.size)
                .sum();
            assert!(used <= a.free_bytes, "account {:?} overcommitted", a.id);
        }
    }
}
