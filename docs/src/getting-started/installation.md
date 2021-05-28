# Installing Dependencies

To get started, make sure to setup all the prerequisite tools on your local machine
(an installer has not yet been developed).

## Install Rust

For an introduction to Rust, see the excellent Rust [book](https://doc.rust-lang.org/book/).

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
rustup component add rustfmt
```

## Install Solana

See the solana [docs](https://docs.solana.com/cli/install-solana-cli-tools) for installation instructions. On macOS and Linux,

```bash
sh -c "$(curl -sSfL https://release.solana.com/v1.6.9/install)"
```

## Install Anchor

Plural is built using [Anchor](https://github.com/project-serum/anchor). To
install the Anchor CLI run,

```bash
cargo install --git https://github.com/project-serum/anchor --tag v0.6.0 anchor-cli --locked
```

