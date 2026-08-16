# Antergos NeXT default zsh config.
# Kept minimal deliberately — shipping this file (rather than none) means
# new users never hit zsh's first-run configuration wizard, which is a
# confusing thing to land on for anyone not expecting it.

if [ "$TERM" = "xterm-kitty" ]; then
    fastfetch
fi
