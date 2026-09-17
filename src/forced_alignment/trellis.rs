#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenSpan {
    pub start_frame: usize,
    pub end_frame: usize,
    pub confidence: f32,
}

const MAX_TRACE_CELLS: usize = 64_000_000;

/// Viterbi alignment over the standard CTC topology:
/// blank, token, blank, token, ... .
pub fn force_align(
    emissions: &[Vec<f32>],
    tokens: &[usize],
    blank: usize,
) -> Result<Vec<TokenSpan>, String> {
    if emissions.is_empty() || tokens.is_empty() {
        return Err("forced alignment requires emissions and transcript tokens".to_owned());
    }
    let vocabulary = emissions[0].len();
    if vocabulary == 0 || emissions.iter().any(|frame| frame.len() != vocabulary) {
        return Err("acoustic emissions have inconsistent vocabulary dimensions".to_owned());
    }
    if blank >= vocabulary || tokens.iter().any(|token| *token >= vocabulary) {
        return Err("transcript token is outside the acoustic-model vocabulary".to_owned());
    }

    let states = tokens
        .len()
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or("forced-alignment transcript is too long")?;
    let cells = emissions
        .len()
        .checked_mul(states)
        .ok_or("forced-alignment input is too long")?;
    if cells > MAX_TRACE_CELLS {
        return Err(format!(
            "forced alignment needs {cells} trace cells (limit {MAX_TRACE_CELLS})"
        ));
    }
    if emissions.len() < tokens.len() {
        return Err("audio has fewer acoustic frames than transcript tokens".to_owned());
    }

    let negative_infinity = f32::NEG_INFINITY;
    let mut previous = vec![negative_infinity; states];
    let mut current = vec![negative_infinity; states];
    let mut trace = vec![0_u8; cells];
    previous[0] = 0.0;

    for (frame_index, logits) in emissions.iter().enumerate() {
        let log_norm = log_sum_exp(logits);
        current.fill(negative_infinity);
        for state in 0..states {
            let label = state_label(state, tokens, blank);
            let emission = logits[label] - log_norm;
            let mut best = previous[state];
            let mut predecessor = 0_u8;
            if state >= 1 && previous[state - 1] > best {
                best = previous[state - 1];
                predecessor = 1;
            }
            if state >= 2
                && state % 2 == 1
                && tokens[state / 2] != tokens[state / 2 - 1]
                && previous[state - 2] > best
            {
                best = previous[state - 2];
                predecessor = 2;
            }
            if best.is_finite() {
                current[state] = best + emission;
                trace[frame_index * states + state] = predecessor;
            }
        }
        std::mem::swap(&mut previous, &mut current);
    }

    let last_token_state = states - 2;
    let mut state = if previous[states - 1] > previous[last_token_state] {
        states - 1
    } else {
        last_token_state
    };
    if !previous[state].is_finite() {
        return Err("could not find a CTC path through the complete lyrics".to_owned());
    }

    let mut starts = vec![usize::MAX; tokens.len()];
    let mut ends = vec![0; tokens.len()];
    let mut confidence_sum = vec![0.0_f32; tokens.len()];
    let mut confidence_count = vec![0_usize; tokens.len()];
    for frame in (0..emissions.len()).rev() {
        if state % 2 == 1 {
            let token_index = state / 2;
            starts[token_index] = starts[token_index].min(frame);
            ends[token_index] = ends[token_index].max(frame + 1);
            let logits = &emissions[frame];
            confidence_sum[token_index] +=
                (logits[tokens[token_index]] - log_sum_exp(logits)).exp();
            confidence_count[token_index] += 1;
        }
        let predecessor = trace[frame * states + state] as usize;
        state = state.saturating_sub(predecessor);
    }

    starts
        .into_iter()
        .zip(ends)
        .zip(confidence_sum.into_iter().zip(confidence_count))
        .enumerate()
        .map(|(index, ((start_frame, end_frame), (sum, count)))| {
            if start_frame == usize::MAX || count == 0 {
                return Err(format!("transcript token {index} was not aligned"));
            }
            Ok(TokenSpan {
                start_frame,
                end_frame,
                confidence: sum / count as f32,
            })
        })
        .collect()
}

fn state_label(state: usize, tokens: &[usize], blank: usize) -> usize {
    if state.is_multiple_of(2) {
        blank
    } else {
        tokens[state / 2]
    }
}

fn log_sum_exp(values: &[f32]) -> f32 {
    let maximum = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    maximum
        + values
            .iter()
            .map(|value| (*value - maximum).exp())
            .sum::<f32>()
            .ln()
}

#[cfg(test)]
mod tests {
    use super::force_align;

    #[test]
    fn aligns_repeated_tokens_through_a_blank() {
        // blank=0, A=1; the only strong path is A, blank, A.
        let emissions = vec![
            vec![0.0, 8.0],
            vec![8.0, 0.0],
            vec![0.0, 8.0],
            vec![8.0, 0.0],
        ];
        let spans = force_align(&emissions, &[1, 1], 0).unwrap();
        assert_eq!(spans[0].start_frame, 0);
        assert_eq!(spans[1].start_frame, 2);
    }
}
