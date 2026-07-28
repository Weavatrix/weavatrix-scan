use super::{IgnoreRule, RuleAction, RuleMatch, RuleScope, RuleSet, RuleTarget};

impl RuleSet {
    pub(super) fn push(&mut self, rule: IgnoreRule) {
        let index = self.rules.len();
        if rule.scope == RuleScope::Anywhere
            && rule.matcher.is_literal()
            && let Some(key) = rule.pattern.bytes().next()
        {
            bucket(&mut self.exact_anywhere, key).push(index);
        } else if let Some(key) = rule.matcher.prefix_key(&rule.pattern) {
            bucket(&mut self.prefixes, key).push(index);
        } else if let Some(key) = rule.matcher.suffix_key(&rule.pattern) {
            bucket(&mut self.suffixes, key).push(index);
        } else {
            self.generic.push(index);
        }
        self.rules.push(rule);
    }

    pub(super) fn matches(&self, path: &str, is_directory: bool) -> Option<RuleMatch> {
        if let Some(action) = self.matches_exact(path, is_directory) {
            return Some(RuleMatch::Exact(action));
        }
        let mut ancestor = path;
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if let Some(action) = self.matches_exact(parent, true) {
                return Some(RuleMatch::Ancestor(action));
            }
            ancestor = parent;
        }
        None
    }

    pub(super) fn matches_exact(&self, path: &str, is_directory: bool) -> Option<RuleAction> {
        let mut best = None;
        let name = path.rsplit('/').next().unwrap_or(path);
        if let Some(indices) = name
            .bytes()
            .next()
            .and_then(|key| bucket_ref(self.exact_anywhere.as_deref(), key))
            && let Some(&index) = indices
                .iter()
                .rev()
                .find(|&&index| self.rules[index].matches_exact(path, is_directory))
        {
            best = Some(index);
        }
        best = self.best_match(&self.generic, path, is_directory, best);
        let name_prefix = name.as_bytes().first();
        if let Some(indices) =
            name_prefix.and_then(|key| bucket_ref(self.prefixes.as_deref(), *key))
        {
            best = self.best_match(indices, path, is_directory, best);
        }
        if let Some(path_prefix) = path.as_bytes().first()
            && Some(path_prefix) != name_prefix
            && let Some(indices) = bucket_ref(self.prefixes.as_deref(), *path_prefix)
        {
            best = self.best_match(indices, path, is_directory, best);
        }
        if let Some(indices) = path
            .as_bytes()
            .last()
            .and_then(|key| bucket_ref(self.suffixes.as_deref(), *key))
        {
            best = self.best_match(indices, path, is_directory, best);
        }
        best.map(|index| self.rules[index].action)
    }

    fn best_match(
        &self,
        indices: &[usize],
        path: &str,
        is_directory: bool,
        mut best: Option<usize>,
    ) -> Option<usize> {
        for &index in indices.iter().rev() {
            if best.is_some_and(|best| index <= best) {
                break;
            }
            if self.rules[index].matches_exact(path, is_directory) {
                best = Some(index);
                break;
            }
        }
        best
    }
}

fn bucket(buckets: &mut Option<Box<[Vec<usize>; 256]>>, key: u8) -> &mut Vec<usize> {
    &mut buckets.get_or_insert_with(|| Box::new(std::array::from_fn(|_| Vec::new())))
        [usize::from(key)]
}

fn bucket_ref(buckets: Option<&[Vec<usize>; 256]>, key: u8) -> Option<&[usize]> {
    buckets.map(|buckets| buckets[usize::from(key)].as_slice())
}

impl IgnoreRule {
    fn matches_exact(&self, path: &str, is_directory: bool) -> bool {
        if self.target == RuleTarget::Directory && !is_directory {
            return false;
        }
        if self.scope == RuleScope::Anywhere {
            let name = path.rsplit('/').next().unwrap_or(path);
            return self.matcher.matches(&self.pattern, name);
        }
        self.matcher.matches(&self.pattern, path)
    }
}
