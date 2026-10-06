use crate::vm::*;

// xoshiro256** seeded by splitmix64; same seed = same numbers on every machine forever
#[derive(Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Rng { s: [next(), next(), next(), next()] }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let out = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }

    // uniform in [0, 1)
    pub fn float(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    // uniform int in [0, n), no modulo bias (Lemire)
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        loop {
            let m = (self.next_u64() as u128) * (n as u128);
            let lo = m as u64;
            if lo >= n.wrapping_neg() % n {
                return (m >> 64) as u64;
            }
        }
    }

    // standard normal, Box-Muller
    pub fn normal(&mut self) -> f64 {
        loop {
            let u = self.float();
            if u > 0.0 {
                let v = self.float();
                return (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos();
            }
        }
    }

    // gamma(shape, 1), Marsaglia-Tsang
    pub fn gamma(&mut self, shape: f64) -> f64 {
        if shape < 1.0 {
            let u = self.float().max(1e-300);
            return self.gamma(shape + 1.0) * u.powf(1.0 / shape);
        }
        let d = shape - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let x = self.normal();
            let v = (1.0 + c * x).powi(3);
            if v <= 0.0 {
                continue;
            }
            let u = self.float();
            if u < 1.0 - 0.0331 * x.powi(4) || u.max(1e-300).ln() < 0.5 * x * x + d * (1.0 - v + v.ln()) {
                return d * v;
            }
        }
    }

    pub fn beta(&mut self, a: f64, b: f64) -> f64 {
        let x = self.gamma(a);
        let y = self.gamma(b);
        x / (x + y)
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            v.swap(i, j);
        }
    }
}

pub static FNS: &[Native] = &[
    Native { name: "seed", f: seed },
    Native { name: "float", f: float },
    Native { name: "int", f: int },
    Native { name: "uniform", f: uniform },
    Native { name: "normal", f: normal },
    Native { name: "choice", f: choice },
    Native { name: "shuffle", f: shuffle },
    Native { name: "sample", f: sample },
    Native { name: "bernoulli", f: bernoulli },
];

fn seed(vm: &mut Vm, a: Args) -> R {
    let [n] = a.bind(["n"])?;
    vm.rng = Rng::new(need(n, "n")?.int("n")? as u64);
    Ok(Value::Nil)
}

fn float(vm: &mut Vm, a: Args) -> R {
    a.bind([])?;
    Ok(Value::Float(vm.rng.float()))
}

// inclusive both ends, like python randint
fn int(vm: &mut Vm, a: Args) -> R {
    let [lo, hi] = a.bind(["lo", "hi"])?;
    let (lo, hi) = (need(lo, "lo")?.int("lo")?, need(hi, "hi")?.int("hi")?);
    if lo > hi {
        return Err(value_err("rand.int() needs lo <= hi"));
    }
    let span = (hi as i128 - lo as i128 + 1) as u128;
    let r = if span > u64::MAX as u128 { vm.rng.next_u64() } else { vm.rng.below(span as u64) };
    Ok(Value::Int((lo as i128 + r as i128) as i64))
}

fn uniform(vm: &mut Vm, a: Args) -> R {
    let [lo, hi] = a.bind(["lo", "hi"])?;
    let (lo, hi) = (need(lo, "lo")?.num("lo")?, need(hi, "hi")?.num("hi")?);
    Ok(Value::Float(lo + (hi - lo) * vm.rng.float()))
}

fn normal(vm: &mut Vm, a: Args) -> R {
    let [mean, sd] = a.bind(["mean", "sd"])?;
    let mean = mean.map_or(Ok(0.0), |v| v.num("mean"))?;
    let sd = sd.map_or(Ok(1.0), |v| v.num("sd"))?;
    Ok(Value::Float(mean + sd * vm.rng.normal()))
}

fn choice(vm: &mut Vm, a: Args) -> R {
    let [xs] = a.bind(["xs"])?;
    let items = super::to_vec(&need(xs, "xs")?, "xs")?;
    if items.is_empty() {
        return Err(value_err("choice() from empty list"));
    }
    Ok(items[vm.rng.below(items.len() as u64) as usize].clone())
}

fn shuffle(vm: &mut Vm, a: Args) -> R {
    let [xs] = a.bind(["xs"])?;
    let xs = need(xs, "xs")?;
    vm.rng.shuffle(&mut xs.as_list("xs")?.borrow_mut());
    Ok(Value::Nil)
}

fn sample(vm: &mut Vm, a: Args) -> R {
    let [xs, k] = a.bind(["xs", "k"])?;
    let mut items = super::to_vec(&need(xs, "xs")?, "xs")?;
    let k = need(k, "k")?.int("k")?;
    if k < 0 || k as usize > items.len() {
        return Err(value_err(format!("sample size {k} is bigger than the list ({})", items.len())));
    }
    vm.rng.shuffle(&mut items);
    items.truncate(k as usize);
    Ok(Value::list(items))
}

fn bernoulli(vm: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["p"])?;
    let p = need(p, "p")?.num("p")?;
    if !(0.0..=1.0).contains(&p) {
        return Err(value_err("p must be between 0 and 1"));
    }
    Ok(Value::Bool(vm.rng.float() < p))
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn stable_and_uniform() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        assert_eq!(a.next_u64(), b.next_u64());
        let mut r = Rng::new(7);
        let n = 100_000;
        let mean: f64 = (0..n).map(|_| r.float()).sum::<f64>() / n as f64;
        assert!((mean - 0.5).abs() < 0.01);
        let nm: f64 = (0..n).map(|_| r.normal()).sum::<f64>() / n as f64;
        assert!(nm.abs() < 0.02);
        assert!((0..1000).all(|_| r.below(6) < 6));
    }
}
