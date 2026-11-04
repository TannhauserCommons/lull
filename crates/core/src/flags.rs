//! Switches for work that lands on main unfinished (ADR-0096). Each is off unless `LULL_FLAGS`
//! names it, so what ships is exactly lull without it. The hub asks `ctx.config.flags.on(&X)`;
//! the web UI gets the names that are on in `/health` and asks `App.flag('name')`.
//!
//! Adding one: a `pub const` [`Flag`] below and its entry in [`FLAGS`], in the first commit of the
//! work. Finishing the work removes both, with the code paths for off.

use std::collections::BTreeSet;

/// One switch: its name in `LULL_FLAGS` and the web UI, what it turns on, and who finishes it.
#[derive(Debug, PartialEq, Eq)]
pub struct Flag {
    pub name: &'static str,
    pub about: &'static str,
    pub owner: &'static str,
}

/// Every switch there is. Empty while nothing is unfinished.
pub const FLAGS: &[&Flag] = &[];

/// The switches that are on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags(BTreeSet<&'static str>);

impl Flags {
    /// `LULL_FLAGS`: names separated by commas. A name that isn't a switch is an error, so a typo
    /// never leaves a feature quietly off.
    pub fn parse(list: &str) -> Result<Flags, String> {
        parse_in(list, FLAGS)
    }

    pub fn on(&self, flag: &Flag) -> bool {
        self.0.contains(flag.name)
    }

    /// The names that are on, for the web UI.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().copied()
    }
}

fn parse_in(list: &str, known: &[&Flag]) -> Result<Flags, String> {
    let mut on = BTreeSet::new();
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let flag = known.iter().find(|f| f.name == name).ok_or_else(|| match known.len() {
            0 => format!("{name:?} is not a switch (there are none now)"),
            _ => format!(
                "{name:?} is not a switch; there are {}",
                known.iter().map(|f| f.name).collect::<Vec<_>>().join(", ")
            ),
        })?;
        on.insert(flag.name);
    }
    Ok(Flags(on))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEEP_CUTS: Flag = Flag {
        name: "deep-cuts",
        about: "a test switch",
        owner: "tests",
    };
    const KNOWN: &[&Flag] = &[&DEEP_CUTS];

    #[test]
    fn a_switch_is_off_unless_named() -> Result<(), String> {
        assert!(!parse_in("", KNOWN)?.on(&DEEP_CUTS));
        let on = parse_in(" deep-cuts ,", KNOWN)?;
        assert!(on.on(&DEEP_CUTS));
        assert_eq!(on.names().collect::<Vec<_>>(), ["deep-cuts"]);
        Ok(())
    }

    #[test]
    fn a_name_that_is_not_a_switch_is_refused() {
        let err = parse_in("deep-cuts,deepcuts", KNOWN).err().unwrap_or_default();
        assert!(err.contains("\"deepcuts\"") && err.contains("deep-cuts"), "{err:?}");
    }

    #[test]
    fn every_switch_has_a_unique_kebab_case_name_and_an_owner() {
        let mut seen = BTreeSet::new();
        for flag in FLAGS {
            assert!(
                !flag.name.is_empty()
                    && flag
                        .name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{:?} is not kebab-case",
                flag.name
            );
            assert!(
                !flag.about.is_empty() && !flag.owner.is_empty(),
                "{} needs about and owner",
                flag.name
            );
            assert!(seen.insert(flag.name), "{} is listed twice", flag.name);
        }
    }
}
