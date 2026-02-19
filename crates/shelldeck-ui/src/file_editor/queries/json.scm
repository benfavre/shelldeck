; JSON highlights (compatible with tree-sitter-json 0.24)

(pair
  key: (string) @property)

(string) @string
(number) @number

[
  (true)
  (false)
] @constant

(null) @constant

["," ":"] @punctuation
["{" "}" "[" "]"] @punctuation
