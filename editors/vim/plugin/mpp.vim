" Vim: register the Muaz++ language server with vim-lsp or ALE if they are installed
if has('nvim') || exists('g:loaded_mpp_plugin')
  finish
endif
let g:loaded_mpp_plugin = 1

if executable('mpp') && get(g:, 'mpp_lsp', 1)
  augroup mpp_lsp
    autocmd!
    autocmd User lsp_setup call lsp#register_server({'name': 'mpp', 'cmd': {_ -> ['mpp', 'lsp']}, 'allowlist': ['mpp']})
  augroup END
  " ALE loads after us; register once everything is up
  autocmd mpp_lsp VimEnter * if exists('g:loaded_ale') | silent! call ale#linter#Define('mpp', {'name': 'mpp', 'lsp': 'stdio', 'executable': 'mpp', 'command': '%e lsp', 'project_root': {b -> fnamemodify(bufname(b), ':p:h')}}) | endif
endif
