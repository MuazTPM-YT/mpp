" Muaz++ buffer settings
if exists('b:did_ftplugin')
  finish
endif
let b:did_ftplugin = 1

setlocal commentstring=#\ %s
setlocal comments=:#
setlocal expandtab shiftwidth=4 softtabstop=4 tabstop=4
setlocal suffixesadd=.mpp
" gq / = with the real formatter
setlocal formatprg=mpp\ fmt\ -
" :make checks the file; errors land in the quickfix list
setlocal makeprg=mpp\ check\ --short\ %
setlocal errorformat=%f:%l:%c:\ error:\ %m

let b:undo_ftplugin = 'setlocal commentstring< comments< expandtab< shiftwidth< softtabstop< tabstop< suffixesadd< formatprg< makeprg< errorformat<'

command! -buffer MppRun  execute '!mpp run ' . shellescape(expand('%'))
command! -buffer MppTest execute '!mpp test ' . shellescape(expand('%'))
command! -buffer MppCheck make
command! -buffer MppFmt  call mpp#format()
