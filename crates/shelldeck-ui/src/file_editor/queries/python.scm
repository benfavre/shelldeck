; Python highlights (compatible with tree-sitter-python 0.25)

(identifier) @variable

(function_definition
  name: (identifier) @function)
(call
  function: (identifier) @function)
(call
  function: (attribute
    attribute: (identifier) @function))

(decorator) @attribute

(class_definition
  name: (identifier) @type)

(parameters
  (identifier) @variable)
(keyword_argument
  name: (identifier) @variable)

(comment) @comment

(string) @string
(interpolation) @escape

(integer) @number
(float) @number

[
  (true)
  (false)
  (none)
] @constant

[
  "+" "-" "*" "/" "//" "%" "**"
  "=" "==" "!=" "<" ">" "<=" ">=" "<<" ">>"
  "&" "|" "^" "~"
  "+=" "-=" "*=" "/=" "//=" "%=" "**=" "&=" "|=" "^=" "<<=" ">>="
  ":=" "->" "@"
] @operator

[
  "(" ")" "[" "]" "{" "}"
] @punctuation

[":" "," "." ";"] @punctuation

[
  "and" "as" "assert" "async" "await" "break" "class" "continue" "def" "del"
  "elif" "else" "except" "finally" "for" "from" "global" "if" "import" "in"
  "is" "lambda" "nonlocal" "not" "or" "pass" "raise" "return" "try" "while"
  "with" "yield"
] @keyword
