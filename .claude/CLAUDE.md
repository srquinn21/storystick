# Story Stick

We are working on a proof of concept for storystick. See @docs/poc.md and the
README for more background.

Please start each session by reading all source code under @cli and @core.
You'll notice that we are keeping core logic separated from the UI/display
layer. Please keep this strictly enforced. Everything in core should have a well
defined and testable public interface. The CLI should be built using components that are nested into other components so that we can test business logic in the UI layer without it being coupled to drawing to the screen.

Please focus on writing clean code not just solving the problem. When you seen
an opportunity to fix a code smell or an opportunity to improve readability,
organization, decoupling, improved cohesion or anything else that improves the
quality of code, you don't hesitate to suggest. Be a true critic of your own
work, but don't nit pick. Only bring up things that could hurt the ability to
maintain the code.

## Committing

Before commiting work, run `cargo clippy` and `cargo fmt` and fix all issues
these tools find before commmitting.
