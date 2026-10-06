# Editor support

All editors use the same language server: `mpp lsp`. It gives errors as you
type, hover docs, completion (including `stats.` style module members), go to
definition (also into `import "file.mpp"`), an outline of functions, classes and
test blocks, and formatting (same as `mpp fmt`).

Make sure `mpp` is on your `PATH` first:

```sh
cargo build --release
cp target/release/mpp ~/.local/bin/
```

## Neovim (0.10+)

The plugin is the `editors/vim` folder. It gives filetype detection, syntax
highlighting, indenting, `:make` (quickfix), formatting, commands, and starts
the language server automatically.

lazy.nvim:

```lua
{ dir = "/path/to/Muaz++/editors/vim", ft = "mpp" }
```

Or without a plugin manager:

```lua
vim.opt.runtimepath:prepend("/path/to/Muaz++/editors/vim")
```

On Neovim 0.11+ the server is set up with `vim.lsp.enable("mpp")` and the
config in `editors/vim/lsp/mpp.lua`. To manage it yourself instead, set
`vim.g.mpp_lsp = false` and call `vim.lsp.enable("mpp")` where you like.

Useful keys after the server attaches (Neovim defaults): `K` hover, `<C-]>`
go to definition, `gO` outline, `<C-x><C-o>` completion, and
`:lua vim.lsp.buf.format()` (or `:MppFmt`) to format.

## Vim (8.2+ / 9)

vim-plug:

```vim
Plug '/path/to/Muaz++/editors/vim'
```

Or native packages:

```sh
mkdir -p ~/.vim/pack/mpp/start
ln -s /path/to/Muaz++/editors/vim ~/.vim/pack/mpp/start/mpp
```

Without any LSP plugin you still get highlighting, indent, and:

| Command | What it does |
|---|---|
| `:make` / `:MppCheck` | check the file; errors go to the quickfix list (`:copen`) |
| `:MppFmt` / `gq` | format with `mpp fmt` |
| `:MppRun` | run the file |
| `:MppTest` | run its test blocks |

With [vim-lsp](https://github.com/prabirshrestha/vim-lsp) or
[ALE](https://github.com/dense-analysis/ale) installed, the plugin registers
`mpp lsp` with them automatically (`let g:mpp_lsp = 0` turns that off).

## VS Code

```sh
cd editors/vscode
npm install
npm run package
code --install-extension muazpp-0.1.0.vsix
```

Settings: `mpp.path` (default `mpp`), `mpp.lsp.enable`. Commands: Muaz++: Run
File, Test File, Test File with HTML Report.

## Other editors

Any editor with LSP support can run `mpp lsp` over stdio for files ending in
`.mpp` (Helix, Zed, Emacs eglot, Sublime LSP, Kate...). Example for Helix
`languages.toml`:

```toml
[language-server.mpp]
command = "mpp"
args = ["lsp"]

[[language]]
name = "mpp"
scope = "source.mpp"
file-types = ["mpp"]
comment-token = "#"
indent = { tab-width = 4, unit = "    " }
language-servers = ["mpp"]
```
