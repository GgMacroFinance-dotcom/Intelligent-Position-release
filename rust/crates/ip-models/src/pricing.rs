//! 模型价格：把一次调用的 token 用量换算成美元，用来执行岗位的每日预算。

use serde::{Deserialize, Serialize};

use crate::Usage;

/// 每百万 token 的美元价格。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    /// 输入（未命中缓存）。
    pub input: f64,
    /// 命中提示前缀缓存的输入；省略时按 `input` 计。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input: Option<f64>,
    /// 写进提示前缀缓存的输入（OpenAI 叫 cache writes，按输入价的 1.25 倍收）；省略时按 `input` 计。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
    pub output: f64,
}

impl Price {
    /// 输入分三份计价：命中缓存的、写进缓存的、其余的。
    pub fn cost(&self, u: &Usage) -> f64 {
        let cached = u.cached_tokens.min(u.prompt_tokens);
        let written = u.cache_write_tokens.min(u.prompt_tokens - cached);
        let fresh = u.prompt_tokens - cached - written;
        (f64::from(fresh) * self.input
            + f64::from(cached) * self.cached_input.unwrap_or(self.input)
            + f64::from(written) * self.cache_write.unwrap_or(self.input)
            + f64::from(u.completion_tokens) * self.output)
            / 1_000_000.0
    }

    pub fn valid(&self) -> bool {
        let ok = |v: f64| v.is_finite() && v >= 0.0;
        ok(self.input) && ok(self.output) && self.cached_input.is_none_or(ok) && self.cache_write.is_none_or(ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_splits_cached_input() {
        let p = Price {
            input: 2.0,
            cached_input: Some(0.5),
            cache_write: None,
            output: 8.0,
        };
        let u = Usage {
            prompt_tokens: 1_000_000,
            completion_tokens: 500_000,
            cached_tokens: 400_000,
            cache_write_tokens: 0,
        };
        // 60 万新输入 × 2 + 40 万缓存 × 0.5 + 50 万输出 × 8
        assert!((p.cost(&u) - (1.2 + 0.2 + 4.0)).abs() < 1e-9);
        let no_cache = Price {
            cached_input: None,
            ..p
        };
        assert!((no_cache.cost(&u) - (2.0 + 4.0)).abs() < 1e-9);
        assert!(!Price { input: -1.0, ..p }.valid());
        assert!(
            !Price {
                cache_write: Some(f64::NAN),
                ..p
            }
            .valid()
        );
    }

    #[test]
    fn cost_charges_cache_writes() {
        // gpt-6-luna 的官方价（2026-11-14）：输入 0.1、命中缓存 0.01、写入缓存 0.125、输出 0.5。
        let luna = Price {
            input: 0.1,
            cached_input: Some(0.01),
            cache_write: Some(0.125),
            output: 0.5,
        };
        // 实测的两次相同请求：第一次 1466 个 token 写进缓存，第二次同样的 1466 个命中。
        let first = Usage {
            prompt_tokens: 1469,
            completion_tokens: 5,
            cached_tokens: 0,
            cache_write_tokens: 1466,
        };
        let second = Usage {
            cached_tokens: 1466,
            cache_write_tokens: 0,
            ..first
        };
        let usd = |fresh: f64, cached: f64, written: f64, out: f64| {
            (fresh * 0.1 + cached * 0.01 + written * 0.125 + out * 0.5) / 1_000_000.0
        };
        assert!((luna.cost(&first) - usd(3.0, 0.0, 1466.0, 5.0)).abs() < 1e-15);
        assert!((luna.cost(&second) - usd(3.0, 1466.0, 0.0, 5.0)).abs() < 1e-15);
        // 没写 cache_write 时写入按普通输入价，和以前一样。
        let plain = Price {
            cache_write: None,
            ..luna
        };
        assert!((plain.cost(&first) - usd(1469.0, 0.0, 0.0, 5.0)).abs() < 1e-15);
        // 报告的数超过总输入时按总输入封顶，不会算出负数。
        let odd = Usage {
            cached_tokens: 1000,
            cache_write_tokens: 1000,
            ..first
        };
        assert!(luna.cost(&odd) > 0.0);
    }
}
