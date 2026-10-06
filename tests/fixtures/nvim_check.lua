-- run by tests/editors.rs inside headless Neovim with editors/vim on the runtimepath
local out = {}
local function say(k, v) table.insert(out, k .. "=" .. tostring(v)) end
vim.cmd("edit " .. vim.g.demo)
say("ft", vim.bo.filetype)
local function syn(l, c) return vim.fn.synIDattr(vim.fn.synID(l, c, 1), "name") end
say("syn", syn(1, 1) .. "," .. syn(2, 4) .. "," .. syn(6, 7))
say("indent", vim.fn.MppIndent(3))
local got = vim.wait(10000, function() return #vim.diagnostic.get(0) > 0 end, 50)
say("diag", got and vim.diagnostic.get(0)[1].lnum or "none")
vim.cmd("MppFmt")
say("fmt", vim.fn.getline(2))
vim.fn.writefile(out, vim.g.result)
vim.cmd("qa!")
