// readability.rs — Flesch-Szigriszt readability index (Spanish)
// Pure computation, no LLM, no I/O.

pub struct ReadabilityResult {
    pub score: f32,
    pub grade: String,
    pub word_count: u32,
    pub sentence_count: u32,
    pub syllable_count: u32,
}

pub fn analyse(text: &str) -> ReadabilityResult {
    let words: Vec<&str> = text.split_whitespace().collect();
    let word_count = words.len() as u32;

    // Count sentences: split on . ! ? followed by space or end
    let sentence_count = text.chars()
        .filter(|&c| c == '.' || c == '!' || c == '?')
        .count()
        .max(1) as u32;

    let syllable_count: u32 = words.iter().map(|w| count_syllables(w)).sum();

    let score = if word_count == 0 {
        0.0
    } else {
        206.835
            - 62.3 * (syllable_count as f32 / word_count as f32)
            - (word_count as f32 / sentence_count as f32)
    };

    let grade = grade_label(score);

    ReadabilityResult { score, grade, word_count, sentence_count, syllable_count }
}

fn count_syllables(word: &str) -> u32 {
    // Heuristic: count vowel groups (a,e,i,o,u + accented variants)
    let vowels = "aeiouáéíóúàèìòùüAEIOUÁÉÍÓÚÀÈÌÒÙÜ";
    let mut count = 0u32;
    let mut prev_was_vowel = false;
    for ch in word.chars() {
        let is_vowel = vowels.contains(ch);
        if is_vowel && !prev_was_vowel {
            count += 1;
        }
        prev_was_vowel = is_vowel;
    }
    count.max(1)
}

fn grade_label(score: f32) -> String {
    match score as u32 {
        90..=100 => "Muy fácil (Educación Primaria)",
        80..=89  => "Bastante fácil (ESO)",
        70..=79  => "Normal (Bachillerato)",
        60..=69  => "Algo difícil (Universitario)",
        50..=59  => "Difícil (Especializado)",
        _        => "Muy difícil (Científico / Jurídico)",
    }.to_string()
}
