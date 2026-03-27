/// Session archetype classification.
///
/// Each recorded session is classified into one of five archetypes based on
/// three signal families:
///
/// | Signal         | Feature          | Source          |
/// |----------------|------------------|-----------------|
/// | Timing rhythm  | mean_gap_ms, cv  | gap_ms column   |
/// | Error pattern  | failure_rate     | exit_code column|
/// | Tool variety   | tool_entropy     | pipeline_steps  |
///
/// ## Archetypes
///
/// | Archetype   | Dominant signal                                      |
/// |-------------|------------------------------------------------------|
/// | Burst       | mean_gap < 2s — scripted, copy-paste, muscle memory  |
/// | Debugging   | failure_rate > 35% — rapid error/fix/retry cycles    |
/// | Focused     | tool_entropy < 0.45 — narrow toolset, specific task  |
/// | Exploratory | tool_entropy ≥ 0.45 — varied tools, open-ended work  |
/// | Unknown     | < 3 commands — insufficient data to classify         |
///
/// Sessions can also be flagged `interrupted` (max_gap ≥ 5 min) independent
/// of the primary archetype — a debugging session can be interrupted.
///
/// ## Design notes
///
/// Rules are ordered so that the most diagnostic signals take precedence:
/// failure_rate before entropy (a high-failure exploration is still Debugging).
/// Burst requires gap data AND ≥ 5 commands (a 2-step fast sequence is noise).
///
/// All thresholds are intentionally conservative; they will be tuned as the
/// corpus grows.

use std::collections::HashMap;

/// Per-session features used for classification.
#[derive(Debug, Clone)]
pub struct SessionFeatures {
    pub session_id:   String,
    pub start_unix:   i64,
    pub shell:        String,
    pub cmd_count:    i64,
    /// Fraction of commands with non-zero exit code.
    pub failure_rate: f64,
    /// Average gap_ms across commands where gap_ms > 0.
    /// 0.0 means no gap data was recorded (pre-8636bc9 sessions).
    pub mean_gap_ms:  f64,
    /// Maximum single gap in the session.
    pub max_gap_ms:   i64,
    /// Coefficient of variation of gap_ms (stddev / mean).
    /// 0.0 if mean_gap_ms == 0.  Reserved for future burst sub-classification.
    #[allow(dead_code)]
    pub gap_cv:       f64,
    /// Normalised Shannon entropy of tool usage: 0 = one tool only, 1 = all tools equal.
    /// 0.0 if no pipeline steps recorded.
    pub tool_entropy: f64,
}

/// Primary session archetype.
#[derive(Debug, Clone, PartialEq)]
pub enum Archetype {
    /// < 3 commands — can't say anything meaningful.
    Unknown,
    /// Rapid-fire commands, mean gap < 2 s.
    Burst,
    /// High failure rate — error/fix/retry loop.
    Debugging,
    /// Low tool entropy — specific, narrow task.
    Focused,
    /// High tool entropy — varied, open-ended work.
    Exploratory,
}


/// Classification result for one session.
pub struct Classification {
    pub archetype:   Archetype,
    /// True if any single gap in the session exceeded 5 minutes.
    pub interrupted: bool,
}

/// Classify a session from its pre-computed features.
pub fn classify(f: &SessionFeatures) -> Classification {
    let interrupted = f.max_gap_ms >= 300_000;

    let archetype = if f.cmd_count < 3 {
        Archetype::Unknown
    } else if f.failure_rate > 0.35 {
        Archetype::Debugging
    } else if f.mean_gap_ms > 0.0 && f.mean_gap_ms < 2_000.0 && f.cmd_count >= 5 {
        Archetype::Burst
    } else if f.tool_entropy > 0.0 && f.tool_entropy < 0.45 {
        Archetype::Focused
    } else if f.tool_entropy >= 0.45 {
        Archetype::Exploratory
    } else {
        // No gap data AND no pipeline entropy (session of bare commands)
        Archetype::Unknown
    };

    Classification { archetype, interrupted }
}

/// Compute normalised Shannon entropy from a (tool, frequency) map.
/// Returns 0.0 for sessions with 0 or 1 unique tools.
pub fn tool_entropy(freqs: &HashMap<String, i64>) -> f64 {
    let total: f64 = freqs.values().map(|&f| f as f64).sum();
    let unique = freqs.len() as f64;
    if total < 2.0 || unique < 2.0 {
        return 0.0;
    }
    let max_h = unique.log2();
    let h: f64 = freqs.values().map(|&f| {
        let p = f as f64 / total;
        -p * p.log2()
    }).sum();
    h / max_h
}

/// Compute gap coefficient of variation from (variance, mean).
/// Returns 0.0 when mean is zero (no gap data).
pub fn gap_cv(variance: f64, mean: f64) -> f64 {
    if mean <= 0.0 || variance <= 0.0 {
        0.0
    } else {
        variance.sqrt() / mean
    }
}

/// Population baseline statistics for archetype features across a corpus of sessions.
/// Used to detect sessions that are statistical outliers (|value − mean| > 2σ).
pub struct BaselineStats {
    pub failure_mean: f64,
    pub failure_sd:   f64,
    pub gap_mean:     f64,  // 0.0 when <3 sessions have gap data
    pub gap_sd:       f64,
    pub entropy_mean: f64,  // 0.0 when <3 sessions have entropy data
    pub entropy_sd:   f64,
}

impl BaselineStats {
    /// Returns Some(true) if failure_rate is >2σ from the mean.
    pub fn failure_outlier(&self, v: f64) -> bool {
        self.failure_sd > 0.0 && (v - self.failure_mean).abs() > 2.0 * self.failure_sd
    }
    /// Returns true if mean_gap_ms is >2σ from the mean (only when v > 0 and sd > 0).
    pub fn gap_outlier(&self, v: f64) -> bool {
        self.gap_sd > 0.0 && v > 0.0 && (v - self.gap_mean).abs() > 2.0 * self.gap_sd
    }
    /// Returns true if tool_entropy is >2σ from the mean (only when v > 0 and sd > 0).
    pub fn entropy_outlier(&self, v: f64) -> bool {
        self.entropy_sd > 0.0 && v > 0.0 && (v - self.entropy_mean).abs() > 2.0 * self.entropy_sd
    }
}

/// Compute population baseline from a slice of classified sessions.
/// Returns None when fewer than 5 sessions are present (corpus too thin for z-scores).
pub fn compute_baseline(pairs: &[(SessionFeatures, Classification)]) -> Option<BaselineStats> {
    if pairs.len() < 5 {
        return None;
    }

    fn mean_sd(vals: &[f64]) -> (f64, f64) {
        if vals.is_empty() {
            return (0.0, 0.0);
        }
        let n = vals.len() as f64;
        let mean = vals.iter().sum::<f64>() / n;
        let var = vals.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        (mean, var.sqrt())
    }

    let failure_vals: Vec<f64> = pairs.iter().map(|(f, _)| f.failure_rate).collect();
    let gap_vals: Vec<f64> = pairs.iter()
        .filter(|(f, _)| f.mean_gap_ms > 0.0)
        .map(|(f, _)| f.mean_gap_ms)
        .collect();
    let entropy_vals: Vec<f64> = pairs.iter()
        .filter(|(f, _)| f.tool_entropy > 0.0)
        .map(|(f, _)| f.tool_entropy)
        .collect();

    let (failure_mean, failure_sd) = mean_sd(&failure_vals);
    let (gap_mean, gap_sd) = if gap_vals.len() >= 3 { mean_sd(&gap_vals) } else { (0.0, 0.0) };
    let (entropy_mean, entropy_sd) = if entropy_vals.len() >= 3 { mean_sd(&entropy_vals) } else { (0.0, 0.0) };

    Some(BaselineStats { failure_mean, failure_sd, gap_mean, gap_sd, entropy_mean, entropy_sd })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feat(cmd_count: i64, failure_rate: f64, mean_gap_ms: f64, max_gap_ms: i64, tool_entropy: f64) -> SessionFeatures {
        SessionFeatures {
            session_id:  String::new(),
            start_unix:  0,
            shell:       String::new(),
            cmd_count,
            failure_rate,
            mean_gap_ms,
            max_gap_ms,
            gap_cv:      0.0,
            tool_entropy,
        }
    }

    #[test]
    fn too_few_commands_is_unknown() {
        assert_eq!(classify(&feat(2, 0.0, 0.0, 0, 0.8)).archetype, Archetype::Unknown);
    }

    #[test]
    fn high_failure_rate_is_debugging() {
        assert_eq!(classify(&feat(10, 0.5, 5000.0, 0, 0.7)).archetype, Archetype::Debugging);
    }

    #[test]
    fn fast_gaps_is_burst() {
        assert_eq!(classify(&feat(8, 0.0, 800.0, 1500, 0.6)).archetype, Archetype::Burst);
    }

    #[test]
    fn burst_requires_five_cmds() {
        // 3 commands with fast gap — too few to call burst
        assert_ne!(classify(&feat(3, 0.0, 800.0, 1500, 0.6)).archetype, Archetype::Burst);
    }

    #[test]
    fn low_entropy_is_focused() {
        assert_eq!(classify(&feat(5, 0.0, 0.0, 0, 0.2)).archetype, Archetype::Focused);
    }

    #[test]
    fn high_entropy_is_exploratory() {
        assert_eq!(classify(&feat(8, 0.1, 0.0, 0, 0.8)).archetype, Archetype::Exploratory);
    }

    #[test]
    fn long_gap_sets_interrupted_flag() {
        let c = classify(&feat(6, 0.0, 0.0, 600_000, 0.7));
        assert!(c.interrupted);
    }

    #[test]
    fn no_gap_data_no_entropy_is_unknown() {
        // Session of bare commands, no pipeline steps, no gap data
        assert_eq!(classify(&feat(5, 0.1, 0.0, 0, 0.0)).archetype, Archetype::Unknown);
    }

    #[test]
    fn entropy_calculation() {
        let mut m = HashMap::new();
        // Uniform distribution: all tools used once → entropy = 1.0
        for t in ["grep", "awk", "sed", "jq"] {
            m.insert(t.to_string(), 1i64);
        }
        let e = tool_entropy(&m);
        assert!((e - 1.0).abs() < 1e-9, "uniform entropy should be 1.0, got {}", e);
    }

    #[test]
    fn entropy_single_tool() {
        let mut m = HashMap::new();
        m.insert("grep".to_string(), 100i64);
        assert_eq!(tool_entropy(&m), 0.0);
    }
}
