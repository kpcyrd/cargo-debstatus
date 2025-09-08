# Making a release

- Update the version in `Cargo.toml`
- Update `Cargo.lock` so it reflects the change in the previous step
    - Either `cargo update` or any other cargo command would do this
- Make sure `cargo build --locked` works
- Commit your changes as e.g. `Release v0.6.5`
- Create a git tag (e.g. `v0.6.5`)
- Run `cargo publish`
- Push your changes with `git push --tags origin main`
- Create a new entry at <https://github.com/kpcyrd/cargo-debstatus/releases>
- Let people in `oftc/#debian-rust` know
