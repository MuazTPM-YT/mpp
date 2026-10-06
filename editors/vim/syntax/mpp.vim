" Vim syntax file
" Language: Muaz++ (.mpp)

if exists('b:current_syntax')
  finish
endif

syn keyword mppKeyword fn return class import as const global nonlocal super
syn keyword mppConditional if elif else
syn keyword mppRepeat while for in break continue
syn keyword mppException try catch throw
syn keyword mppOperatorWord and or not
syn keyword mppBoolean true false
syn keyword mppNil nil
syn keyword mppSelf self

" test blocks are words only at the start of a line, before a string
syn match mppTestBlock "^\s*\zs\(test\|experiment\|bench\|property\)\ze\s\+[\"']"
syn match mppExpect "^\s*\zs\(expect\|report\)\>\ze\s*[^=, ]"
syn match mppExpect "\<within\>"

syn keyword mppBuiltin print len type str repr int float bool list range abs min max sum round
syn keyword mppBuiltin sorted reversed enumerate zip any all input error assert env args exit warn
syn keyword mppBuiltin chr ord copy isinstance callable skip expect_throws expect_snapshot note
syn keyword mppBuiltin vec linspace table load_csv load_jsonl load_json plot
syn keyword mppModule math time io json rand gen stats ab power bandit ml llm

" later rules win on ties, so calls go first and definitions override them
syn match mppCall "\<[A-Za-z_][A-Za-z0-9_]*\ze("
syn match mppFunction "\(\<fn\s\+\)\@<=[A-Za-z_][A-Za-z0-9_]*"
syn match mppClass "\(\<class\s\+\)\@<=[A-Za-z_][A-Za-z0-9_]*"

syn match mppNumber "\<\d[0-9_]*\(\.\d[0-9_]*\)\=\([eE][+-]\=\d\+\)\=\>"
syn match mppNumber "\<0[xX][0-9a-fA-F_]\+\>"
syn match mppNumber "\<0[bB][01_]\+\>"
syn match mppNumber "\<0[oO][0-7_]\+\>"

syn match mppEscape "\\[ntr0\\\"'{}]" contained
syn match mppEscape "\\u{\x\+}" contained
syn region mppInterp matchgroup=mppInterpDelim start="{" end="}" contained contains=TOP
syn match mppInterpBrace "{{\|}}" contained

syn region mppString start=+"""+ end=+"""+ contains=mppEscape
syn region mppString start=+"+ skip=+\\.+ end=+"+ oneline contains=mppEscape
syn region mppString start=+'+ skip=+\\.+ end=+'+ oneline contains=mppEscape
syn region mppRawString start=+r"+ end=+"+ oneline
syn region mppRawString start=+r'+ end=+'+ oneline
syn region mppFString start=+f"""+ end=+"""+ contains=mppEscape,mppInterpBrace,mppInterp
syn region mppFString start=+f"+ skip=+\\.+ end=+"+ oneline contains=mppEscape,mppInterpBrace,mppInterp
syn region mppFString start=+f'+ skip=+\\.+ end=+'+ oneline contains=mppEscape,mppInterpBrace,mppInterp

syn match mppOperator "=>\|\.\.=\|\.\.\|\*\*\|//\|\~=\|[-+*/%<>=!?]=\?"

syn keyword mppTodo TODO FIXME XXX NOTE contained
syn match mppComment "#.*$" contains=mppTodo,@Spell

hi def link mppKeyword Keyword
hi def link mppConditional Conditional
hi def link mppRepeat Repeat
hi def link mppException Exception
hi def link mppOperatorWord Operator
hi def link mppBoolean Boolean
hi def link mppNil Constant
hi def link mppSelf Identifier
hi def link mppTestBlock Statement
hi def link mppExpect Special
hi def link mppBuiltin Function
hi def link mppModule Include
hi def link mppFunction Function
hi def link mppClass Type
hi def link mppCall Function
hi def link mppNumber Number
hi def link mppString String
hi def link mppRawString String
hi def link mppFString String
hi def link mppEscape SpecialChar
hi def link mppInterpDelim Delimiter
hi def link mppInterpBrace SpecialChar
hi def link mppOperator Operator
hi def link mppComment Comment
hi def link mppTodo Todo

let b:current_syntax = 'mpp'
