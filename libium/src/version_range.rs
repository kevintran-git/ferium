use std::cmp::Ordering;

#[derive(Debug, Clone)]
enum Predicate {
    Any,
    Eq(String),
    Ge(String),
    Le(String),
    Gt(String),
    Lt(String),
    Caret(String),
    Tilde(String),
    Wildcard(String),
}

impl Predicate {
    fn matches(&self, version: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Eq(v) => compare_versions(version, v) == Ordering::Equal,
            Self::Ge(v) => compare_versions(version, v) != Ordering::Less,
            Self::Le(v) => compare_versions(version, v) != Ordering::Greater,
            Self::Gt(v) => compare_versions(version, v) == Ordering::Greater,
            Self::Lt(v) => compare_versions(version, v) == Ordering::Less,
            Self::Caret(v) => {
                if compare_versions(version, v) == Ordering::Less {
                    return false;
                }
                let (v_nums, _) = split_version(v);
                let (ver_nums, _) = split_version(version);
                let bump_idx = v_nums.iter().position(|&n| n != 0).unwrap_or(0);
                (0..bump_idx).all(|i| get(&ver_nums, i) == get(&v_nums, i))
                    && get(&ver_nums, bump_idx) == get(&v_nums, bump_idx)
            }
            Self::Tilde(v) => {
                if compare_versions(version, v) == Ordering::Less {
                    return false;
                }
                let (v_nums, _) = split_version(v);
                let (ver_nums, _) = split_version(version);
                let fixed_len = if v_nums.len() >= 2 { v_nums.len() - 1 } else { v_nums.len() };
                (0..fixed_len).all(|i| get(&ver_nums, i) == get(&v_nums, i))
            }
            Self::Wildcard(prefix) => {
                let (prefix_nums, _) = split_version(prefix);
                let (ver_nums, _) = split_version(version);
                prefix_nums.len() <= ver_nums.len()
                    && prefix_nums.iter().enumerate().all(|(i, &n)| get(&ver_nums, i) == n)
            }
        }
    }
}

fn get(nums: &[u64], i: usize) -> u64 {
    if i < nums.len() {
        nums[i]
    } else {
        0
    }
}

fn split_version(v: &str) -> (Vec<u64>, String) {
    let v = v.trim();
    let v = v.strip_prefix(['v', 'V']).unwrap_or(v);
    let core = v.split('+').next().unwrap_or(v);

    let mut nums = Vec::new();
    let mut rest = String::new();
    let mut parts = core.split('.').peekable();
    while let Some(part) = parts.next() {
        let digit_len = part.chars().take_while(char::is_ascii_digit).count();
        if digit_len == 0 {
            rest.push_str(part);
            for remaining in parts {
                rest.push('.');
                rest.push_str(remaining);
            }
            break;
        }
        nums.push(part[..digit_len].parse().unwrap_or(0));
        if digit_len < part.len() {
            rest.push_str(&part[digit_len..]);
            for remaining in parts {
                rest.push('.');
                rest.push_str(remaining);
            }
            break;
        }
    }
    (nums, rest)
}

fn compare_versions(a: &str, b: &str) -> Ordering {
    let (a_nums, a_rest) = split_version(a);
    let (b_nums, b_rest) = split_version(b);
    let len = a_nums.len().max(b_nums.len());
    for i in 0..len {
        match get(&a_nums, i).cmp(&get(&b_nums, i)) {
            Ordering::Equal => {}
            ord => return ord,
        }
    }
    match (a_rest.is_empty(), b_rest.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a_rest.cmp(&b_rest),
    }
}

fn parse_predicate(token: &str) -> Predicate {
    let token = token.trim();
    if token == "*" {
        return Predicate::Any;
    }
    for (prefix, ctor) in [
        (">=", Predicate::Ge as fn(String) -> Predicate),
        ("<=", Predicate::Le),
        (">", Predicate::Gt),
        ("<", Predicate::Lt),
        ("^", Predicate::Caret),
        ("~", Predicate::Tilde),
        ("=", Predicate::Eq),
    ] {
        if let Some(rest) = token.strip_prefix(prefix) {
            return ctor(rest.trim().to_string());
        }
    }
    if let Some(prefix) = token
        .strip_suffix(".x")
        .or_else(|| token.strip_suffix(".X"))
        .or_else(|| token.strip_suffix(".*"))
    {
        return Predicate::Wildcard(prefix.to_string());
    }
    Predicate::Eq(token.to_string())
}

fn parse_predicate_group(s: &str) -> Vec<Predicate> {
    let predicates = s.split_whitespace().map(parse_predicate).collect::<Vec<_>>();
    if predicates.is_empty() {
        vec![Predicate::Any]
    } else {
        predicates
    }
}

#[derive(Debug, Clone)]
pub struct Requirement {
    groups: Vec<Vec<Predicate>>,
}

impl Requirement {
    pub fn any() -> Self {
        Self {
            groups: vec![vec![Predicate::Any]],
        }
    }

    pub fn satisfies(&self, version: &str) -> bool {
        self.groups
            .iter()
            .any(|group| group.iter().all(|p| p.matches(version)))
    }

    pub fn parse_fabric(value: &serde_json::Value) -> Self {
        let groups = match value {
            serde_json::Value::String(s) => vec![parse_predicate_group(s)],
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(parse_predicate_group)
                .collect(),
            _ => vec![],
        };
        if groups.is_empty() {
            Self::any()
        } else {
            Self { groups }
        }
    }

    pub fn parse_maven(range: &str) -> Self {
        let range = range.trim();
        let bracketed = range.len() >= 2
            && (range.starts_with('[') || range.starts_with('('))
            && (range.ends_with(']') || range.ends_with(')'));
        if !bracketed {
            return Self::any();
        }

        let lower_inclusive = range.starts_with('[');
        let upper_inclusive = range.ends_with(']');
        let inner = &range[1..range.len() - 1];

        if !inner.contains(',') {
            let version = inner.trim();
            return if version.is_empty() {
                Self::any()
            } else {
                Self {
                    groups: vec![vec![Predicate::Eq(version.to_string())]],
                }
            };
        }

        let Some((lo, hi)) = inner.split_once(',') else {
            return Self::any();
        };
        let (lo, hi) = (lo.trim(), hi.trim());

        let mut group = Vec::new();
        if !lo.is_empty() {
            group.push(if lower_inclusive {
                Predicate::Ge(lo.to_string())
            } else {
                Predicate::Gt(lo.to_string())
            });
        }
        if !hi.is_empty() {
            group.push(if upper_inclusive {
                Predicate::Le(hi.to_string())
            } else {
                Predicate::Lt(hi.to_string())
            });
        }

        if group.is_empty() {
            Self::any()
        } else {
            Self { groups: vec![group] }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Requirement;
    use serde_json::json;

    #[test]
    fn fabric_simple_operators() {
        let req = Requirement::parse_fabric(&json!(">=1.2.0"));
        assert!(req.satisfies("1.2.0"));
        assert!(req.satisfies("1.5.0"));
        assert!(!req.satisfies("1.1.9"));
    }

    #[test]
    fn fabric_and_group() {
        let req = Requirement::parse_fabric(&json!(">=1.2.0 <1.3.0"));
        assert!(req.satisfies("1.2.5"));
        assert!(!req.satisfies("1.3.0"));
        assert!(!req.satisfies("1.1.0"));
    }

    #[test]
    fn fabric_or_array() {
        let req = Requirement::parse_fabric(&json!(["1.2.0", "1.4.0"]));
        assert!(req.satisfies("1.2.0"));
        assert!(req.satisfies("1.4.0"));
        assert!(!req.satisfies("1.3.0"));
    }

    #[test]
    fn fabric_caret_stays_within_major() {
        let req = Requirement::parse_fabric(&json!("^1.2.3"));
        assert!(req.satisfies("1.2.3"));
        assert!(req.satisfies("1.9.0"));
        assert!(!req.satisfies("2.0.0"));
        assert!(!req.satisfies("1.2.2"));
    }

    #[test]
    fn fabric_caret_zero_major_is_stricter() {
        let req = Requirement::parse_fabric(&json!("^0.2.3"));
        assert!(req.satisfies("0.2.9"));
        assert!(!req.satisfies("0.3.0"));
    }

    #[test]
    fn fabric_wildcard() {
        let req = Requirement::parse_fabric(&json!("1.2.x"));
        assert!(req.satisfies("1.2.0"));
        assert!(req.satisfies("1.2.99"));
        assert!(!req.satisfies("1.3.0"));
    }

    #[test]
    fn fabric_any_is_unconstrained() {
        let req = Requirement::parse_fabric(&serde_json::Value::Null);
        assert!(req.satisfies("0.0.1"));
        assert!(req.satisfies("999.0.0"));
    }

    #[test]
    fn maven_half_open_interval() {
        let req = Requirement::parse_maven("[1.0,2.0)");
        assert!(req.satisfies("1.0"));
        assert!(req.satisfies("1.9.9"));
        assert!(!req.satisfies("2.0"));
        assert!(!req.satisfies("0.9"));
    }

    #[test]
    fn maven_open_lower_bound() {
        let req = Requirement::parse_maven("[1.5,)");
        assert!(req.satisfies("1.5"));
        assert!(req.satisfies("100.0"));
        assert!(!req.satisfies("1.4.9"));
    }

    #[test]
    fn maven_exact_version() {
        let req = Requirement::parse_maven("[1.0]");
        assert!(req.satisfies("1.0"));
        assert!(!req.satisfies("1.0.1"));
    }

    #[test]
    fn maven_unbracketed_is_a_soft_recommendation() {
        let req = Requirement::parse_maven("1.0");
        assert!(req.satisfies("0.1"));
        assert!(req.satisfies("50.0"));
    }

    #[test]
    fn maven_empty_range_is_unconstrained() {
        let req = Requirement::parse_maven("");
        assert!(req.satisfies("anything"));
    }
}
