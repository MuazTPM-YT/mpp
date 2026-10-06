" Muaz++ indent: one level per open bracket, closers dedent
if exists('b:did_indent')
  finish
endif
let b:did_indent = 1

setlocal indentexpr=MppIndent(v:lnum)
setlocal indentkeys=0{,0},0),0],!^F,o,O,e,0.
setlocal autoindent

let b:undo_indent = 'setlocal indentexpr< indentkeys< autoindent<'

if exists('*MppIndent')
  finish
endif

" code part of a line: strings and comments removed
function! s:Code(line) abort
  let l:s = substitute(a:line, '"\(\\.\|[^"\\]\)*"', '""', 'g')
  let l:s = substitute(l:s, "'\\(\\\\.\\|[^'\\\\]\\)*'", "''", 'g')
  return substitute(l:s, '#.*$', '', '')
endfunction

function! MppIndent(lnum) abort
  let l:prev = prevnonblank(a:lnum - 1)
  if l:prev == 0
    return 0
  endif
  let l:p = s:Code(getline(l:prev))
  let l:ind = indent(l:prev)
  " opened minus closed brackets on the previous line
  let l:opens = len(substitute(l:p, '[^{([]', '', 'g'))
  let l:closes = len(substitute(l:p, '[^})\]]', '', 'g'))
  " a line that starts with closers already dedented itself
  let l:lead = len(matchstr(l:p, '^\s*\zs[})\]]\+'))
  let l:net = l:opens - (l:closes - l:lead)
  if l:net > 0
    let l:ind += shiftwidth()
  elseif l:net < 0
    let l:ind -= shiftwidth()
  endif
  " method-chain lines (.filter(...)) sit one level deeper than the start
  let l:cur = getline(a:lnum)
  if l:p =~# '^\s*\.' && l:cur !~# '^\s*\.'
    let l:ind -= shiftwidth()
  elseif l:cur =~# '^\s*\.' && l:p !~# '^\s*\.'
    let l:ind += shiftwidth()
  endif
  if l:cur =~# '^\s*[})\]]'
    let l:ind -= shiftwidth()
  endif
  return max([l:ind, 0])
endfunction
