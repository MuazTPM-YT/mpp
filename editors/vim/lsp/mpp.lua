-- Neovim 0.11+: picked up by vim.lsp.config / vim.lsp.enable("mpp")
return {
  cmd = { "mpp", "lsp" },
  filetypes = { "mpp" },
  root_markers = { ".git", ".env.example" },
}
