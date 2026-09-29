# Operation-local native binding

Describes the native call boundary that accepts sealed `operation/local`
evidence and a weak handle from the live checker owner. The call returns an
opaque qualified value; it cannot serialize, restore, or extend authority.
