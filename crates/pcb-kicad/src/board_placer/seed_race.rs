// Copyright (C) 2026 Toit contributors.

//! The placement seeds raced in a pool, and the pool's work as the
//! reference machine sees it.

/// The threads of the reference machine the placement race's seeds count
/// as running on (a pool this wide, whatever the process's threads). The
/// router's `EXPANSIONS_PER_SECOND` is calibrated on one search thread;
/// this pool width is the placement's own convention.
pub(crate) const REFERENCE_THREADS: usize = 8;

/// A placement race's outcome (`race_seeds`).
pub(crate) struct SeedRace<T> {
    /// The kept seeds' offsets and results, in seed order.
    pub kept: Vec<(u64, T)>,
    /// How many seeds ran (the kept ones and those past the budget).
    pub run: u64,
    /// The kept seeds' work as `REFERENCE_THREADS` threads would run each
    /// batch (`pool_makespan`), summed over the batches.
    pub pool_work: u64,
}

/// How long `threads` threads take for `works`, each starting in turn on
/// the thread that frees first.
pub(crate) fn pool_makespan(works: &[u64], threads: usize) -> u64 {
    let mut free = vec![0u64; threads.max(1)];
    for work in works {
        let earliest = (0..free.len())
            .min_by_key(|thread| free[*thread])
            .unwrap_or(0);
        free[earliest] += work;
    }
    free.into_iter().max().unwrap_or(0)
}

/// Runs seeds `0..seeds` of a race in a pool of `threads` and keeps them as
/// running one after the other would: seed k is kept while the work of the
/// seeds before it is under `limit` (the first `minimum` always). The
/// seeds run in batches, each in a pool where a seed starts as a thread
/// frees: the first holds the `minimum`; each later one as many seeds as
/// the budget left holds at the kept seeds' mean work (all that are left
/// without a limit). The batches, so the seeds run and the ones kept,
/// follow from the seeds' work alone, whatever `threads` and whichever
/// seed finishes first; seeds a batch ran past the budget count for
/// nothing.
pub(crate) fn race_seeds<T: Send>(
    seeds: u64,
    minimum: u64,
    limit: Option<u64>,
    threads: usize,
    work: impl Fn(&T) -> u64,
    run: impl Fn(u64) -> T + Sync,
) -> SeedRace<T> {
    let mut race = SeedRace {
        kept: Vec::new(),
        run: 0,
        pool_work: 0,
    };
    let mut used = 0u64;
    let mut next = 0u64;
    while next < seeds {
        let batch = match limit {
            _ if race.kept.is_empty() => minimum.max(1),
            None => seeds - next,
            Some(limit) => {
                let mean = (used / race.kept.len() as u64).max(1);
                limit.saturating_sub(used).div_ceil(mean).max(1)
            }
        }
        .min(seeds - next);
        let results = run_pool(next..next + batch, threads, &run);
        race.run += batch;
        let mut batch_works = Vec::new();
        let mut spent = false;
        for (offset, result) in (next..).zip(results) {
            if race.kept.len() as u64 >= minimum && limit.is_some_and(|limit| used >= limit) {
                spent = true;
                break;
            }
            let own = work(&result);
            used += own;
            batch_works.push(own);
            race.kept.push((offset, result));
        }
        race.pool_work += pool_makespan(&batch_works, REFERENCE_THREADS);
        next += batch;
        // The next seed would not be kept either: it does not run.
        if spent || (race.kept.len() as u64 >= minimum && limit.is_some_and(|limit| used >= limit))
        {
            break;
        }
    }
    race
}

/// Runs `run` on every index of `range` with `threads` threads, each taking
/// the next index as it frees; the results in index order.
fn run_pool<T: Send>(
    range: std::ops::Range<u64>,
    threads: usize,
    run: &(impl Fn(u64) -> T + Sync),
) -> Vec<T> {
    let count = (range.end - range.start) as usize;
    let next = std::sync::atomic::AtomicU64::new(range.start);
    let slots: Vec<std::sync::Mutex<Option<T>>> =
        (0..count).map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, count.max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index >= range.end {
                        break;
                    }
                    let result = run(index);
                    *slots[(index - range.start) as usize]
                        .lock()
                        .expect("placement slot") = Some(result);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .expect("placement slot")
                .expect("placement thread")
        })
        .collect()
}
