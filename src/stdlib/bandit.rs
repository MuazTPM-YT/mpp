// multi-armed bandits: live objects + offline simulation
use super::rand::Rng;
use super::stats::tests::Rec;
use super::stats::{num_or, nums};
use crate::vm::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
enum Policy {
    Thompson,
    Ucb1,
    Epsilon(f64),
}

struct State {
    pulls: Vec<f64>,
    reward: Vec<f64>,
}

pub const METHODS: &[&str] = &["choose", "update", "stats", "prob_best"];

pub struct Bandit {
    policy: Policy,
    st: RefCell<State>,
}

impl Bandit {
    fn new(policy: Policy, k: usize) -> Bandit {
        Bandit { policy, st: RefCell::new(State { pulls: vec![0.0; k], reward: vec![0.0; k] }) }
    }

    fn choose(&self, rng: &mut Rng) -> usize {
        let st = self.st.borrow();
        let k = st.pulls.len();
        if let Some(i) = st.pulls.iter().position(|p| *p == 0.0)
            && self.policy != Policy::Thompson
        {
            return i;
        }
        match self.policy {
            Policy::Thompson => {
                (0..k)
                    .map(|i| {
                        // Beta(1 + wins, 1 + losses); rewards clamp to [0, 1]
                        let w = st.reward[i].clamp(0.0, st.pulls[i]);
                        (i, rng.beta(1.0 + w, 1.0 + st.pulls[i] - w))
                    })
                    .fold((0, f64::MIN), |b, x| if x.1 > b.1 { x } else { b })
                    .0
            }
            Policy::Ucb1 => {
                let total: f64 = st.pulls.iter().sum();
                (0..k)
                    .map(|i| (i, st.reward[i] / st.pulls[i] + (2.0 * total.ln() / st.pulls[i]).sqrt()))
                    .fold((0, f64::MIN), |b, x| if x.1 > b.1 { x } else { b })
                    .0
            }
            Policy::Epsilon(e) => {
                if rng.float() < e {
                    rng.below(k as u64) as usize
                } else {
                    (0..k).map(|i| (i, st.reward[i] / st.pulls[i])).fold((0, f64::MIN), |b, x| if x.1 > b.1 { x } else { b }).0
                }
            }
        }
    }

    fn update(&self, arm: usize, r: f64) {
        let mut st = self.st.borrow_mut();
        st.pulls[arm] += 1.0;
        st.reward[arm] += r;
    }
}

impl Object for Bandit {
    fn type_name(&self) -> &'static str {
        "bandit"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn call_method(&self, vm: &mut Vm, _this: &Value, name: &str, a: Args) -> R {
        let k = self.st.borrow().pulls.len();
        match name {
            "choose" => {
                a.bind([])?;
                Ok(Value::Int(self.choose(&mut vm.rng) as i64))
            }
            "update" => {
                let [arm, r] = a.bind(["arm", "reward"])?;
                let arm = need(arm, "arm")?.int("arm")?;
                if arm < 0 || arm as usize >= k {
                    return Err(value_err(format!("arm must be 0..{}", k - 1)));
                }
                let r = need(r, "reward")?.num("reward")?;
                if self.policy == Policy::Thompson && !(0.0..=1.0).contains(&r) {
                    return Err(value_err("thompson bandit rewards must be between 0 and 1"));
                }
                self.update(arm as usize, r);
                Ok(Value::Nil)
            }
            "stats" => {
                a.bind([])?;
                let st = self.st.borrow();
                let arms: Vec<Value> = (0..k)
                    .map(|i| {
                        Rec::default()
                            .int("arm", i as i64)
                            .int("pulls", st.pulls[i] as i64)
                            .num("mean_reward", if st.pulls[i] > 0.0 { st.reward[i] / st.pulls[i] } else { f64::NAN })
                            .value()
                    })
                    .collect();
                Ok(Value::list(arms))
            }
            "prob_best" => {
                let [draws] = a.bind(["draws"])?;
                let draws = draws.map_or(Ok(10_000), |v| v.int("draws"))?.clamp(100, 1_000_000) as usize;
                let st = self.st.borrow();
                let mut wins = vec![0usize; k];
                for _ in 0..draws {
                    let best = (0..k)
                        .map(|i| {
                            let w = st.reward[i].clamp(0.0, st.pulls[i]);
                            (i, vm.rng.beta(1.0 + w, 1.0 + st.pulls[i] - w))
                        })
                        .fold((0, f64::MIN), |b, x| if x.1 > b.1 { x } else { b })
                        .0;
                    wins[best] += 1;
                }
                Ok(Value::list(wins.into_iter().map(|w| Value::Float(w as f64 / draws as f64)).collect()))
            }
            _ => Err(err("AttributeError", format!("bandit has no method `{name}`"))),
        }
    }
}

fn policy_of(name: &str, eps: f64) -> Result<Policy, Flow> {
    match name {
        "thompson" => Ok(Policy::Thompson),
        "ucb1" | "ucb" => Ok(Policy::Ucb1),
        "epsilon" | "epsilon_greedy" => Ok(Policy::Epsilon(eps)),
        other => Err(value_err(format!("unknown policy {other:?} (use thompson, ucb1, epsilon)"))),
    }
}

fn arms_arg(v: Option<Value>) -> Result<usize, Flow> {
    let k = need(v, "arms")?.int("arms")?;
    if !(2..=10_000).contains(&k) {
        return Err(value_err("arms must be between 2 and 10000"));
    }
    Ok(k as usize)
}

fn make(p: Policy, k: usize) -> R {
    Ok(Value::Object(Rc::new(Bandit::new(p, k))))
}

pub static FNS: &[Native] = &[
    Native {
        name: "thompson",
        f: |_, a| {
            let [k] = a.bind(["arms"])?;
            make(Policy::Thompson, arms_arg(k)?)
        },
    },
    Native {
        name: "ucb1",
        f: |_, a| {
            let [k] = a.bind(["arms"])?;
            make(Policy::Ucb1, arms_arg(k)?)
        },
    },
    Native {
        name: "epsilon_greedy",
        f: |_, a| {
            let [k, e] = a.bind(["arms", "epsilon"])?;
            make(Policy::Epsilon(num_or(e, "epsilon", 0.1)?), arms_arg(k)?)
        },
    },
    Native { name: "simulate", f: simulate },
];

// play a policy against known Bernoulli arms; report regret
fn simulate(vm: &mut Vm, a: Args) -> R {
    let [arms, policy, steps, eps, runs] = a.bind(["arms", "policy", "steps", "epsilon", "runs"])?;
    let p = nums(arms, "arms")?;
    if p.len() < 2 || p.iter().any(|x| !(0.0..=1.0).contains(x)) {
        return Err(value_err("arms must be 2+ success rates between 0 and 1"));
    }
    let name = opt(policy).map_or(Ok("thompson".to_string()), |v| Ok::<_, Flow>(v.as_str("policy")?.to_string()))?;
    let pol = policy_of(&name, num_or(eps, "epsilon", 0.1)?)?;
    let steps = opt(steps).map_or(Ok(10_000), |v| v.int("steps"))?.clamp(1, 100_000_000) as usize;
    let runs = opt(runs).map_or(Ok(1), |v| v.int("runs"))?.clamp(1, 10_000) as usize;
    let best = p.iter().copied().fold(f64::MIN, f64::max);
    let best_arm = p.iter().position(|x| *x == best).unwrap_or(0);
    let k = p.len();
    let mut pulls = vec![0.0; k];
    let mut total_reward = 0.0;
    let mut regret = 0.0;
    let marks = 20usize.min(steps);
    let mut curve = vec![0.0; marks];
    for _ in 0..runs {
        let b = Bandit::new(pol, k);
        let mut run_regret = 0.0;
        for s in 0..steps {
            let arm = b.choose(&mut vm.rng);
            let r = (vm.rng.float() < p[arm]) as i64 as f64;
            b.update(arm, r);
            pulls[arm] += 1.0;
            total_reward += r;
            run_regret += best - p[arm];
            if (s + 1) % (steps / marks).max(1) == 0 && (s + 1) / (steps / marks).max(1) <= marks {
                curve[(s + 1) / (steps / marks).max(1) - 1] += run_regret;
            }
        }
        regret += run_regret;
    }
    let rf = runs as f64;
    Ok(Rec::default()
        .text("policy", &name)
        .val("pulls", Value::list(pulls.iter().map(|x| Value::Float(x / rf)).collect()))
        .val("share", Value::list(pulls.iter().map(|x| Value::Float(x / (rf * steps as f64))).collect()))
        .num("mean_reward", total_reward / (rf * steps as f64))
        .num("regret", regret / rf)
        .int("best_arm", best_arm as i64)
        .val("regret_curve", Value::list(curve.into_iter().map(|c| Value::Float(c / rf)).collect()))
        .int("steps", steps as i64)
        .int("runs", runs as i64)
        .value())
}
