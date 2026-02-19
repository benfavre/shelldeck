; Bash/Shell highlights (compatible with tree-sitter-bash 0.25)

(command_name) @function

(function_definition
  name: (word) @function)

(comment) @comment

(string) @string
(raw_string) @string
(heredoc_body) @string
(heredoc_start) @string
(concatenation) @string

(variable_name) @variable
(special_variable_name) @variable
(simple_expansion) @variable
(expansion) @variable

(command_substitution) @escape

[
  "if" "then" "else" "elif" "fi"
  "case" "esac"
  "for" "while" "until" "do" "done" "in"
  "function"
] @keyword

[
  "=" "==" "!=" "<" ">"
  "|" "||" "&&" "!"
  ";;" "&"
  ">>" "<<" ">&" "<&" "+="
] @operator

[
  "(" ")" "[" "]" "{" "}" "[[" "]]"
] @punctuation

[";" ","] @punctuation
