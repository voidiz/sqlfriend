use rustyline::validate::{
    MatchingBracketValidator, ValidationContext, ValidationResult, Validator,
};

use sqlfriend_core::command::command_prefix;

#[derive(Default)]
pub(crate) struct ReadlineValidator {
    bracket_validator: MatchingBracketValidator,
    statement_validator: StatementValidator,
}

impl Validator for ReadlineValidator {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        let bracket_result = self.bracket_validator.validate(ctx);
        let statement_result = self.statement_validator.validate(ctx);

        chain_validation_result(bracket_result, statement_result)
    }
}

/// If first is valid, return second. Otherwise return first. Discards the message of first.
fn chain_validation_result(
    first: rustyline::Result<ValidationResult>,
    second: rustyline::Result<ValidationResult>,
) -> rustyline::Result<ValidationResult> {
    if let Ok(ValidationResult::Valid(_)) = first {
        return second;
    };

    first
}

/// Validate that a line ends with a semicolon.
#[derive(Default)]
struct StatementValidator {}

impl Validator for StatementValidator {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        Ok(validate_statement(ctx.input()))
    }
}

fn validate_statement(input: &str) -> ValidationResult {
    let chars = input.chars().collect::<String>();
    if chars.starts_with(command_prefix!()) {
        return ValidationResult::Valid(None);
    }

    match input.chars().last() {
        Some(char) => {
            if char == ';' {
                ValidationResult::Valid(None)
            } else {
                ValidationResult::Incomplete
            }
        }
        None => ValidationResult::Incomplete,
    }
}
