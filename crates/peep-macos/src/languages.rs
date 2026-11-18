use anyhow::{Context, Result};
use scriptrs::{LanguageSelection, ParakeetLanguage};

pub const CHOICES: &[(&str, &str)] = &[
    ("bg", "Bulgarian"),
    ("hr", "Croatian"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("nl", "Dutch"),
    ("en", "English"),
    ("et", "Estonian"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("de", "German"),
    ("el", "Greek"),
    ("hu", "Hungarian"),
    ("it", "Italian"),
    ("lv", "Latvian"),
    ("lt", "Lithuanian"),
    ("mt", "Maltese"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("es", "Spanish"),
    ("sv", "Swedish"),
    ("uk", "Ukrainian"),
];

pub fn selection(code: Option<&str>) -> Result<LanguageSelection> {
    match code {
        None => Ok(LanguageSelection::Auto),
        Some(code) => Ok(LanguageSelection::Language(
            ParakeetLanguage::from_code(code).context("Unsupported dictation language.")?,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_languages_match_the_models_supported_languages() {
        assert_eq!(CHOICES.len(), scriptrs::V3_LANGUAGES.len());
        for (code, _) in CHOICES {
            assert!(selection(Some(code)).is_ok());
        }
        assert_eq!(selection(None).unwrap(), LanguageSelection::Auto);
        assert_eq!(
            selection(Some("en")).unwrap(),
            LanguageSelection::Language(ParakeetLanguage::English)
        );
        assert!(selection(Some("invalid")).is_err());
    }
}
