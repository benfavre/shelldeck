; JavaScript highlights (compatible with tree-sitter-javascript 0.25)

(identifier) @variable

(property_identifier) @property

(function_expression
  name: (identifier) @function)
(function_declaration
  name: (identifier) @function)
(method_definition
  name: (property_identifier) @function)

(call_expression
  function: (identifier) @function)
(call_expression
  function: (member_expression
    property: (property_identifier) @function))

(this) @variable
(super) @variable

[
  (true)
  (false)
  (null)
  (undefined)
] @constant

(comment) @comment

[
  (string)
  (template_string)
] @string

(regex) @string
(number) @number

[
  ";"
  "."
  ","
] @punctuation

[
  "-" "--" "-="
  "+" "++" "+="
  "*" "*=" "**" "**="
  "/" "/="
  "%" "%="
  "<" "<=" "<<" "<<="
  "=" "==" "===" "!" "!=" "!=="
  "=>" ">" ">=" ">>" ">>=" ">>>" ">>>="
  "~" "^" "&" "|"
  "^=" "&=" "|="
  "&&" "||" "??" "&&=" "||=" "??="
] @operator

[
  "(" ")" "[" "]" "{" "}"
] @punctuation

[
  "as" "async" "await" "break" "case" "catch" "class" "const" "continue"
  "debugger" "default" "delete" "do" "else" "export" "extends" "finally"
  "for" "from" "function" "get" "if" "import" "in" "instanceof" "let"
  "new" "of" "return" "set" "static" "switch" "throw" "try" "typeof"
  "var" "void" "while" "with" "yield"
] @keyword
