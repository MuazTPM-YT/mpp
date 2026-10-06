-- Neovim: .mpp is Muaz++ (not cpp), and start its language server (vim.g.mpp_lsp = false to opt out)
vim.filetype.add({ extension = { mpp = "mpp" } })

if vim.g.mpp_lsp == false or vim.fn.executable("mpp") == 0 then
  return
end
if vim.lsp.enable then
  vim.lsp.enable("mpp")
else
  -- Neovim 0.10 and older
  vim.api.nvim_create_autocmd("FileType", {
    pattern = "mpp",
    callback = function(args)
      vim.lsp.start({ name = "mpp", cmd = { "mpp", "lsp" }, root_dir = vim.fs.root(args.buf, { ".git" }) or vim.fn.getcwd() })
    end,
  })
end
