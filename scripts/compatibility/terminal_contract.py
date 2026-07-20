#!/usr/bin/env python3
"""Purpose: name the complete terminal compatibility scenario contract.
Owns: stable terminal scenario identifiers and their automated expected results.
Must not: launch a terminal, execute scenarios, write evidence, or contact a network.
Invariants: automated and manual runners emit every identifier in TERMINAL_SCENARIOS.
Phase: post-v0.1 Linux compatibility matrix.
"""

TERMINAL_EXPECTATIONS = {
    "core-open-edit-save-quit": "Open, edit, save, and quit produce the exact fixture with exit status 0.",
    "input-delivery": "UTF-8 and control-key input reaches Catomic without rewriting.",
    "shifted-text": "Uppercase letters and shifted punctuation are saved exactly.",
    "fallback-function-keys": "F1 opens help and F2 opens the command prompt without editing.",
    "mouse-mapping": "An SGR click maps to the expected document row and column.",
    "bracketed-paste": "A bracketed UTF-8 paste is one undoable and redoable edit.",
    "osc52": "A real terminal clipboard read returns the exact bounded OSC 52 copy.",
    "resize": "Smaller and larger dimensions trigger renders without file changes or failure.",
    "signals": "SIGINT exits through the signal path and restores terminal modes.",
    "terminal-restoration": "Clean exit disables mouse/paste modes and leaves the alternate screen.",
}
TERMINAL_SCENARIOS = tuple(TERMINAL_EXPECTATIONS)


def terminal_expected(identifier: str) -> str:
    return TERMINAL_EXPECTATIONS[identifier]
