//! "Up to N examples, and the count of what they stand for" — one collector
//! (RFC 09 §5.1 O6).
//!
//! Every judge in this crate names a handful of offenders and then says how
//! many more there were: the doctor's per-check findings, `field`'s per-path
//! ones, `expect`'s violations, `cutover`'s leaked keys, `budget`'s example
//! expansions, `why`'s evidence lines. It was written out by hand at each
//! site, with a different cap and a slightly different closure, and the deep
//! review found what that costs: the `qos-observed-mismatch` cap capped the
//! *candidates* rather than the findings, so violators past the first twenty
//! judged keys vanished **and** the remainder note under-counted them.
//!
//! The bug is structural — a hand-written cap counts whatever it happens to
//! be looking at — so the fix is one collector that counts everything offered
//! and keeps the first `cap`. The cap **value** stays each site's own policy;
//! only the mechanism is shared.

/// A bounded example list that remembers how many it turned away.
///
/// `total` counts every item offered, `len` how many are held: the two are
/// what makes "… and N more" honest, and neither can drift from the other
/// because nothing else feeds them.
#[derive(Debug, Clone)]
pub struct Examples<T> {
    kept: Vec<T>,
    total: usize,
    cap: usize,
}

impl<T> Examples<T> {
    /// A collector holding at most `cap` examples.
    pub fn new(cap: usize) -> Self {
        Examples {
            kept: Vec::new(),
            total: 0,
            cap,
        }
    }

    /// Take up to `cap` items from `iter`, counting the rest.
    ///
    /// The whole iterator is consumed — that is where `total` comes from — so
    /// pass a cheap one. An item that costs an allocation to build belongs in
    /// [`push_with`](Self::push_with), which does not build it past the cap.
    pub fn collect(cap: usize, iter: impl IntoIterator<Item = T>) -> Self {
        let mut ex = Examples::new(cap);
        for item in iter {
            ex.push(item);
        }
        ex
    }

    /// Offer one example: counted always, kept while there is room.
    pub fn push(&mut self, item: T) {
        self.total += 1;
        if self.kept.len() < self.cap {
            self.kept.push(item);
        }
    }

    /// Offer one example, building it only if it will be kept.
    ///
    /// The same accounting as [`push`](Self::push) — the total counts the
    /// call, not the closure — for the sites where the example is a
    /// `format!` over a population that can be large.
    pub fn push_with(&mut self, make: impl FnOnce() -> T) {
        self.total += 1;
        if self.kept.len() < self.cap {
            self.kept.push(make());
        }
    }

    /// Every item offered, kept or not.
    pub fn total(&self) -> usize {
        self.total
    }

    /// How many were held back by the cap.
    pub fn dropped(&self) -> usize {
        self.total - self.kept.len()
    }

    pub fn len(&self) -> usize {
        self.kept.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.kept
    }

    pub fn into_vec(self) -> Vec<T> {
        self.kept
    }

    /// The remainder line — `… and {dropped} {tail}` — or `None` when the cap
    /// never bit. `tail` is the site's own wording, which is why it is a
    /// parameter: this unifies the arithmetic, not the sentence.
    pub fn more(&self, tail: &str) -> Option<String> {
        (self.dropped() > 0).then(|| format!("… and {} {tail}", self.dropped()))
    }
}

impl Examples<String> {
    /// The kept lines, followed by the remainder line when the cap bit.
    pub fn into_lines(self, tail: &str) -> Vec<String> {
        let more = self.more(tail);
        let mut lines = self.into_vec();
        lines.extend(more);
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug that motivated the extraction: what is counted must be what
    /// was *offered*, not what happened to be kept.
    #[test]
    fn the_total_counts_everything_offered() {
        let mut ex = Examples::new(3);
        for i in 0..10 {
            ex.push(format!("k{i}"));
        }
        assert_eq!(ex.len(), 3);
        assert_eq!(ex.total(), 10);
        assert_eq!(ex.dropped(), 7);
        assert_eq!(ex.as_slice()[0], "k0", "the first offered are the kept");
        assert_eq!(
            ex.more("more key(s) with the same finding").as_deref(),
            Some("… and 7 more key(s) with the same finding")
        );
    }

    /// Under the cap there is no remainder to name, and nothing is dropped.
    #[test]
    fn an_uncapped_run_says_nothing_extra() {
        let ex = Examples::collect(20, (0..3).map(|i| format!("k{i}")));
        assert_eq!(ex.total(), 3);
        assert_eq!(ex.dropped(), 0);
        assert_eq!(ex.more("more"), None);
        assert_eq!(ex.into_lines("more").len(), 3);
    }

    /// `push_with` accounts like `push` but does not build past the cap.
    #[test]
    fn push_with_counts_the_call_not_the_closure() {
        let mut built = 0;
        let mut ex = Examples::new(2);
        for _ in 0..5 {
            ex.push_with(|| {
                built += 1;
                String::from("x")
            });
        }
        assert_eq!(built, 2, "only the kept were built");
        assert_eq!(ex.total(), 5, "all five were counted");
        assert_eq!(ex.into_lines("more")[2], "… and 3 more");
    }
}
