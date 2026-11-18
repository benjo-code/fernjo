//! Pure planner that decides which authentication attempt to try next. No I/O.

/// Hard cap on authentication attempts per connection (servers commonly allow six).
pub const MAX_TOTAL: usize = 6;
const MAX_AGENT: usize = 5;
const MAX_TYPED: usize = 3;

/// What this side can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Available {
    pub agent_identities: usize,
    pub has_key: bool,
    pub has_password: bool,
}

/// Methods the server currently accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offered {
    pub publickey: bool,
    pub password: bool,
    pub keyboard_interactive: bool,
}

/// One authentication attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    Agent(usize),
    CredentialKey,
    CredentialPassword,
    KeyboardInteractive,
    TypedPassword,
}

/// Hands out attempts in the order agent, key, stored password, keyboard-interactive, typed password.
///
/// Budget: `MAX_TOTAL = 6`. `reserved = has_key + has_password + 1 (keyboard-interactive) + 1 (typed
/// password)`. Agent attempts = `min(agent_identities, 5, MAX_TOTAL - reserved)`, never below 0.
/// Example: 10 agent identities, key and password present: reserved = 1 + 1 + 1 + 1 = 4, so
/// `min(10, 5, 6 - 4)` = 2 agent attempts. Without credentials: reserved = 2, so 4 agent attempts.
/// Typed password attempts = `min(3, MAX_TOTAL - attempts already returned)`, at least one while
/// any budget remains.
#[derive(Debug)]
pub struct AuthPlanner {
    plan: Vec<Attempt>,
    used: Vec<bool>,
    returned: usize,
    typed_returned: usize,
}

impl AuthPlanner {
    pub fn new(available: Available) -> Self {
        let reserved = usize::from(available.has_key) + usize::from(available.has_password) + 2;
        let agents = available
            .agent_identities
            .min(MAX_AGENT)
            .min(MAX_TOTAL.saturating_sub(reserved));
        let mut plan: Vec<Attempt> = (0..agents).map(Attempt::Agent).collect();
        if available.has_key {
            plan.push(Attempt::CredentialKey);
        }
        if available.has_password {
            plan.push(Attempt::CredentialPassword);
        }
        plan.push(Attempt::KeyboardInteractive);
        plan.extend(std::iter::repeat_n(Attempt::TypedPassword, MAX_TYPED));
        let used = vec![false; plan.len()];
        Self {
            plan,
            used,
            returned: 0,
            typed_returned: 0,
        }
    }

    /// Next unused attempt the server's current `offered` set allows, or `None`.
    pub fn next(&mut self, offered: Offered) -> Option<Attempt> {
        if self.returned >= MAX_TOTAL {
            return None;
        }
        for (i, attempt) in self.plan.iter().enumerate() {
            if self.used[i] {
                continue;
            }
            let allowed = match attempt {
                Attempt::Agent(_) | Attempt::CredentialKey => offered.publickey,
                Attempt::CredentialPassword => offered.password,
                Attempt::KeyboardInteractive => offered.keyboard_interactive,
                Attempt::TypedPassword => offered.password && self.typed_returned < MAX_TYPED,
            };
            if allowed {
                self.used[i] = true;
                self.returned += 1;
                if *attempt == Attempt::TypedPassword {
                    self.typed_returned += 1;
                }
                return Some(*attempt);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Offered = Offered {
        publickey: true,
        password: true,
        keyboard_interactive: true,
    };

    fn drain(p: &mut AuthPlanner, offered: Offered) -> Vec<Attempt> {
        std::iter::from_fn(|| p.next(offered)).collect()
    }

    #[test]
    fn order_is_agent_key_password_kbdint_typed() {
        let mut p = AuthPlanner::new(Available {
            agent_identities: 2,
            has_key: true,
            has_password: true,
        });
        assert_eq!(
            drain(&mut p, ALL),
            [
                Attempt::Agent(0),
                Attempt::Agent(1),
                Attempt::CredentialKey,
                Attempt::CredentialPassword,
                Attempt::KeyboardInteractive,
                Attempt::TypedPassword
            ]
        );
        assert_eq!(p.next(ALL), None);
    }

    #[test]
    fn skips_methods_the_server_does_not_offer() {
        let kbd_only = Offered {
            publickey: false,
            password: false,
            keyboard_interactive: true,
        };
        let avail = Available {
            agent_identities: 2,
            has_key: true,
            has_password: true,
        };
        let mut p = AuthPlanner::new(avail);
        assert_eq!(drain(&mut p, kbd_only), [Attempt::KeyboardInteractive]);

        let no_password = Offered {
            publickey: true,
            password: false,
            keyboard_interactive: true,
        };
        let mut p = AuthPlanner::new(avail);
        let got = drain(&mut p, no_password);
        assert!(!got.contains(&Attempt::TypedPassword));
        assert!(!got.contains(&Attempt::CredentialPassword));
    }

    #[test]
    fn ten_agent_identities_are_capped_by_the_budget() {
        let count_agents =
            |v: &[Attempt]| v.iter().filter(|a| matches!(a, Attempt::Agent(_))).count();

        let mut p = AuthPlanner::new(Available {
            agent_identities: 10,
            has_key: true,
            has_password: true,
        });
        assert_eq!(count_agents(&drain(&mut p, ALL)), 2);

        let mut p = AuthPlanner::new(Available {
            agent_identities: 10,
            has_key: false,
            has_password: false,
        });
        let got = drain(&mut p, ALL);
        assert_eq!(count_agents(&got), 4);
        assert_eq!(
            &got[4..],
            [Attempt::KeyboardInteractive, Attempt::TypedPassword]
        );
    }

    #[test]
    fn typed_password_gets_three_tries_when_budget_allows() {
        let mut p = AuthPlanner::new(Available {
            agent_identities: 0,
            has_key: false,
            has_password: false,
        });
        assert_eq!(
            drain(&mut p, ALL),
            [
                Attempt::KeyboardInteractive,
                Attempt::TypedPassword,
                Attempt::TypedPassword,
                Attempt::TypedPassword
            ]
        );
    }

    #[test]
    fn offered_set_may_change_between_calls() {
        let mut p = AuthPlanner::new(Available {
            agent_identities: 1,
            has_key: true,
            has_password: false,
        });
        let pk = Offered {
            publickey: true,
            password: false,
            keyboard_interactive: false,
        };
        assert_eq!(p.next(pk), Some(Attempt::Agent(0)));
        // Partial success: the server now wants only keyboard-interactive.
        let kbd = Offered {
            publickey: false,
            password: false,
            keyboard_interactive: true,
        };
        assert_eq!(p.next(kbd), Some(Attempt::KeyboardInteractive));
        assert_eq!(p.next(kbd), None);
    }

    #[test]
    fn nothing_available_and_nothing_offered_is_none() {
        let mut p = AuthPlanner::new(Available {
            agent_identities: 0,
            has_key: false,
            has_password: false,
        });
        let none = Offered {
            publickey: false,
            password: false,
            keyboard_interactive: false,
        };
        assert_eq!(p.next(none), None);
    }
}
