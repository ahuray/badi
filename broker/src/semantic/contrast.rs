//! The contrastive first token (broker README, contrastive first token).
//!
//! Gemma 3 1B's English misses were generic lead-ins: its most likely first
//! token is often the one any small model would also write. Among the first
//! tokens it rates at least a tenth as likely as its best, and no less likely
//! than the confidence floor, the one maximising
//! `log p(writer) − 0.5 · log p(contrast)` starts the suggestion; the writer
//! continues it greedily. The contrast model (Gemma 3 270M) shares the
//! writer's vocabulary and never writes text itself.
//!
//! On 100 held-out English prefixes, blind-reviewed with the production
//! floor applied, this showed 55 useful suggestions against greedy's 46
//! (12 wins, 3 losses) and no harmful one (2026-11-18).

/// The writer's first-token candidates the choice is made among.
pub(crate) const WRITER_CANDIDATES: u8 = 10;
/// The contrast model's first tokens scored; a writer candidate outside
/// them counts as just below the least likely of them.
pub(crate) const CONTRAST_SCORES: u16 = 200;
/// `ln 0.1`: a candidate must be at least a tenth as likely as the best.
const HEAD: f64 = -std::f64::consts::LN_10;
/// The weight of the contrast model's log-probability.
const WEIGHT: f64 = 0.5;
/// The part of the writing budget the contrast reads leave to generate the
/// suggestion: eight tokens from a cached prompt took about 250 ms on the
/// measured laptop. Reads still running then are dropped, and the request
/// runs greedy.
pub(crate) const GENERATION_RESERVE: std::time::Duration = std::time::Duration::from_millis(250);

/// One first token and its pre-sampling log-probability.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScoredToken {
    /// The vocabulary id, when the runtime reported it.
    pub id: Option<u64>,
    pub text: String,
    pub logprob: f64,
}

/// The first token to start the suggestion with when the contrastive choice
/// differs from the writer's own best, else `None`. A token qualifies only
/// when it extends `echo` with something other than spaces, carries none of
/// the characters the generation grammar excludes, and no digit unless
/// `digits`; `floor` is the confidence floor every shown token must meet.
/// The choice itself is never a number.
pub(crate) fn contrastive_choice<'a>(
    writer: &'a [ScoredToken],
    contrast: &[ScoredToken],
    echo: &str,
    digits: bool,
    floor: f64,
) -> Option<&'a str> {
    let admissible: Vec<&ScoredToken> = writer
        .iter()
        .filter(|token| admissible(&token.text, echo, digits) && token.logprob.is_finite())
        .collect();
    // The writer's own pick is its most likely admissible token: the one the
    // grammar-constrained greedy request would start with.
    let greedy = admissible.iter().copied().reduce(|best, token| {
        if token.logprob > best.logprob {
            token
        } else {
            best
        }
    })?;
    let least = contrast
        .iter()
        .map(|token| token.logprob)
        .filter(|logprob| logprob.is_finite())
        .reduce(f64::min)?;
    let contrast_logprob = |token: &ScoredToken| {
        contrast
            .iter()
            .find(|scored| match (scored.id, token.id) {
                (Some(left), Some(right)) => left == right,
                _ => scored.text == token.text,
            })
            .map(|scored| scored.logprob)
            .filter(|logprob| logprob.is_finite())
            .unwrap_or(least - 1.0)
    };
    // A number is never forced: one the typed text lacks fails the fact
    // check whole, and the contrast model favours rare ones. When the
    // writer's own pick is a number, its request stays greedy.
    let numeric = |token: &ScoredToken| token.text.chars().any(char::is_numeric);
    if numeric(greedy) {
        return None;
    }
    let cut = (greedy.logprob + HEAD).max(floor);
    let choice = admissible
        .iter()
        .copied()
        .filter(|token| token.logprob >= cut && !numeric(token))
        .map(|token| (token, token.logprob - WEIGHT * contrast_logprob(token)))
        // The earlier (more likely) token wins a tie.
        .reduce(|best, next| if next.1 > best.1 { next } else { best })?
        .0;
    (choice.text != greedy.text).then_some(choice.text.as_str())
}

fn admissible(text: &str, echo: &str, digits: bool) -> bool {
    text.len() > echo.len()
        && text.starts_with(echo)
        && !text[echo.len()..].trim().is_empty()
        && !text.contains(['<', '>', '`', '\n', '\r'])
        && (digits || !text.chars().any(char::is_numeric))
}

#[cfg(test)]
mod tests;
