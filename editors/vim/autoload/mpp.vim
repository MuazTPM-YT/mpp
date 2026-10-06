" format the whole buffer with `mpp fmt -`; keep the cursor; leave buffer alone on errors
function! mpp#format() abort
  let l:view = winsaveview()
  let l:out = systemlist('mpp fmt -', getline(1, '$'))
  if v:shell_error
    echohl ErrorMsg | echom 'mpp fmt: ' . join(l:out, ' ') | echohl None
    return
  endif
  if l:out !=# getline(1, '$')
    silent! undojoin
    call setline(1, l:out)
    if line('$') > len(l:out)
      silent execute (len(l:out) + 1) . ',$delete _'
    endif
  endif
  call winrestview(l:view)
endfunction
