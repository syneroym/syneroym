#!/bin/sh

# The PATH for git hooks is not always the same as your interactive shell.
# GUI-based git clients, for example, might not source your .bashrc or .zshrc.
# To ensure that 'cargo' can be found, we'll explicitly add the default
# cargo installation directory to the PATH.
export PATH="$HOME/.cargo/bin:$PATH"

# Ensure formatting is correct
echo "Checking cargo fmt..."
if ! cargo +nightly fmt --all -- --check; then
    echo "❌ cargo fmt failed! Please format your code using 'cargo +nightly fmt'."
    exit 1
fi

# Run clippy to catch common mistakes
echo "Checking cargo clippy..."
if ! cargo clippy --workspace --all-targets --all-features -- -D warnings; then
    echo "❌ cargo clippy failed! Please fix the warnings before committing."
    exit 1
fi

echo "✅ All pre-commit checks passed!"
exit 0
