[
  "@startuml"
  "@enduml"
  "include"
] @keyword

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

"," @punctuation.delimiter
"=" @operator

(procedure
  (procedure_identifier) @function.call)

((procedure
  (procedure_identifier) @type)
  (#any-of? @type
    "Person"
    "Person_Ext"
    "System"
    "System_Ext"
    "SystemDb"
    "Container"
    "ContainerDb"
    "Component"
    "ComponentDb"
    "Boundary"
    "Enterprise_Boundary"
    "System_Boundary"
    "Container_Boundary"))

((procedure
  (procedure_identifier) @keyword)
  (#any-of? @keyword
    "Rel"
    "Rel_R"
    "Rel_L"
    "Rel_U"
    "Rel_D"
    "BiRel"
    "Lay_R"
    "Lay_L"
    "Lay_U"
    "Lay_D"))

((procedure
  (procedure_identifier) @function.special)
  (#any-of? @function.special
    "LAYOUT_WITH_LEGEND"
    "LAYOUT_TOP_DOWN"
    "LAYOUT_LEFT_RIGHT"
    "SHOW_LEGEND"
    "HIDE_STEREOTYPE"))

(string) @string
(single_quote_string) @string
(unqouted_string) @string
(identifier) @constant
(link) @link_uri

(preprocessor
  url: (unqouted_string) @link_uri) @keyword.import
