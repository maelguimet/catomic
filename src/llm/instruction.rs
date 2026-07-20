//! Purpose: this file must parse explicit `>>> catomic` instruction blocks.
//! Owns: block delimiter recognition, source line ranges, and parse errors.
//! Must not: read buffers, collect repo context, construct clients, or mutate text.
//! Invariants: delimiters occupy whole trimmed lines; returned ranges are zero-based.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstructionBlock {
    pub start_line: usize,
    pub end_line: usize,
    pub instruction: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstructionParseError {
    NestedStart { line: usize },
    UnexpectedEnd { line: usize },
    UnclosedBlock { start_line: usize },
}

pub fn parse_instruction_blocks(
    text: &str,
) -> Result<Vec<InstructionBlock>, InstructionParseError> {
    let mut blocks = Vec::new();
    let mut open: Option<(usize, Vec<&str>)> = None;

    for (line_index, line) in text.lines().enumerate() {
        match line.trim() {
            ">>> catomic" => {
                if open.is_some() {
                    return Err(InstructionParseError::NestedStart { line: line_index });
                }
                open = Some((line_index, Vec::new()));
            }
            "<<<" => {
                let Some((start_line, lines)) = open.take() else {
                    return Err(InstructionParseError::UnexpectedEnd { line: line_index });
                };
                blocks.push(InstructionBlock {
                    start_line,
                    end_line: line_index,
                    instruction: lines.join("\n"),
                });
            }
            _ => {
                if let Some((_, lines)) = open.as_mut() {
                    lines.push(line);
                }
            }
        }
    }

    if let Some((start_line, _)) = open {
        return Err(InstructionParseError::UnclosedBlock { start_line });
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_blocks_and_preserves_instruction_text() {
        let text = "before\n>>> catomic\nRefactor this.\n  Keep indentation.  \n<<<\nafter\n\
                    >>> catomic\nwrite tests\n<<<\n";

        assert_eq!(
            parse_instruction_blocks(text),
            Ok(vec![
                InstructionBlock {
                    start_line: 1,
                    end_line: 4,
                    instruction: "Refactor this.\n  Keep indentation.  ".to_string(),
                },
                InstructionBlock {
                    start_line: 6,
                    end_line: 8,
                    instruction: "write tests".to_string(),
                },
            ])
        );
    }

    #[test]
    fn delimiters_must_occupy_trimmed_whole_lines() {
        let text = "prefix >>> catomic\n>>> catomic extra\ntext <<<\n";
        assert_eq!(parse_instruction_blocks(text), Ok(Vec::new()));
    }

    #[test]
    fn accepts_indented_delimiters_and_empty_instruction() {
        let text = "  >>> catomic  \n\t<<<\n";
        assert_eq!(
            parse_instruction_blocks(text),
            Ok(vec![InstructionBlock {
                start_line: 0,
                end_line: 1,
                instruction: String::new(),
            }])
        );
    }

    #[test]
    fn rejects_nested_unexpected_and_unclosed_delimiters() {
        assert_eq!(
            parse_instruction_blocks(">>> catomic\n>>> catomic\n<<<"),
            Err(InstructionParseError::NestedStart { line: 1 })
        );
        assert_eq!(
            parse_instruction_blocks("text\n<<<"),
            Err(InstructionParseError::UnexpectedEnd { line: 1 })
        );
        assert_eq!(
            parse_instruction_blocks("text\n>>> catomic\nwork"),
            Err(InstructionParseError::UnclosedBlock { start_line: 1 })
        );
    }
}
