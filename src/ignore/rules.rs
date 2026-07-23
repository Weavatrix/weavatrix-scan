use super::{IgnoreRule, RuleAction, RuleMatch, RuleScope, RuleSet, RuleTarget};

impl RuleSet {
    pub(super) fn push(&mut self, rule: IgnoreRule) {
        let index = self.rules.len();
        if rule.scope == RuleScope::Anywhere && rule.matcher.is_literal() {
            self.exact_anywhere
                .entry(rule.pattern.clone())
                .or_default()
                .push(index);
        } else if let Some(key) = rule.matcher.prefix_key() {
            self.prefixes.entry(key).or_default().push(index);
        } else if let Some(key) = rule.matcher.suffix_key(&rule.pattern) {
            self.suffixes.entry(key).or_default().push(index);
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

    fn matches_exact(&self, path: &str, is_directory: bool) -> Option<RuleAction> {
        let mut best = None;
        let name = path.rsplit('/').next().unwrap_or(path);
        if let Some(indices) = self.exact_anywhere.get(name)
            && let Some(&index) = indices
                .iter()
                .rev()
                .find(|&&index| self.rules[index].matches_exact(path, is_directory))
        {
            best = Some(index);
        }
        best = self.best_match(&self.generic, path, is_directory, best);
        let name_prefix = name.as_bytes().first();
        if let Some(indices) = name_prefix.and_then(|key| self.prefixes.get(key)) {
            best = self.best_match(indices, path, is_directory, best);
        }
        if let Some(path_prefix) = path.as_bytes().first()
            && Some(path_prefix) != name_prefix
            && let Some(indices) = self.prefixes.get(path_prefix)
        {
            best = self.best_match(indices, path, is_directory, best);
        }
        if let Some(indices) = path
            .as_bytes()
            .last()
            .and_then(|key| self.suffixes.get(key))
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
