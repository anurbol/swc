use super::strip;
use crate::resolver;
use swc_common::chain;

macro_rules! to {
    ($name:ident, $from:expr, $to:expr) => {
        test!(
            ::swc_ecma_parser::Syntax::Typescript(Default::default()),
            |_| strip(),
            $name,
            $from,
            $to
        );
    };
}

to!(
    constructor_01,
    "class Foo {
    constructor(public readonly foo) {}
}",
    "class Foo {
    constructor(foo) {
        this.foo = foo;
    }
}"
);

to!(
    constructor_02,
    "class Foo {
    constructor(readonly foo) {
        this.bar = 1;
    }
}",
    "class Foo {
    constructor(foo) {
        this.foo = foo;
        this.bar = 1;
    }
}"
);

to!(export_import, "export import A = B", "export { B as A }");

to!(export_equals, "export = Foo", "export default Foo");

to!(
    issue_196_01,
    "export type Link = { key: string; text: string };",
    ""
);

to!(
    issue_196_02,
    "type Link = { key: string; text: string };
export { Link };",
    ""
);

to!(
    issue_196_03,
    "type Link = { key: string; text: string };
const Link = 'Boo';
export { Link };",
    "const Link = 'Boo';
export { Link };"
);

// TODO: Test function / variable hoisting

to!(
    issue_179_01,
    "import {Types} from 'other';
const a: Types.foo = {};",
    "import 'other';
const a = {};"
);

to!(
    issue_179_02,
    "import {Types} from 'other';
const a: Types = Types.foo;",
    "import {Types} from 'other';
const a = Types.foo;"
);

to!(
    issue_236,
    "function foo(this: any, $scope: angular.IScope){}",
    "function foo($scope){}"
);

to!(
    issue_357,
    "export function addProp<T, K extends string, V>(
  obj: T,
  prop: K,
  value: V
): T & { [x in K]: V };
export function addProp<T, K extends string, V>(
  prop: K,
  value: V
): (obj: T) => T & { [x in K]: V };

export function addProp(arg1: any, arg2: any, arg3?: any): any {
  if (arguments.length === 2) {
    return (object: any) => _addProp(object, arg1, arg2);
  }
  return _addProp(arg1, arg2, arg3);
}

function _addProp(obj: any, prop: string, value: any) {
  return {
    ...obj,
    [prop]: value,
  };
}",
    "export function addProp(arg1, arg2, arg3) {
  if (arguments.length === 2) {
    return (object) => _addProp(object, arg1, arg2);
  }
  return _addProp(arg1, arg2, arg3);
}

function _addProp(obj, prop, value) {
  return {
    ...obj,
    [prop]: value,
  };
}
"
);

to!(
    issue_366_01,
    "
class App {
  public enter?(): void;
  public leave?(): void;
  public destroy?(): void;
}",
    "class App {}"
);

to!(
    issue_366_02,
    "
function enter(): string;
function enter(foo: string): number;
",
    ""
);

to!(
    issue_392_1,
    "
import { PlainObject } from 'simplytyped';
const dict: PlainObject = {};
",
    "
import 'simplytyped';
const dict = {};"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| chain!(strip(), resolver()),
    issue_392_2,
    "
import { PlainObject } from 'simplytyped';
const dict: PlainObject = {};
",
    "
import 'simplytyped';
const dict = {};"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    issue_461,
    "for (let x in ['']) {
  (x => 0)(x);
}",
    "for(let x in ['']){
    ((x)=>0
    )(x);
}"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    issue_468_1,
    "tView.firstCreatePass ?
      getOrCreateTNode(tView, lView[T_HOST], index, TNodeType.Element, null, null) :
      tView.data[adjustedIndex] as TElementNode",
    "tView.firstCreatePass ? getOrCreateTNode(tView, lView[T_HOST], index, TNodeType.Element, \
     null, null) : tView.data[adjustedIndex];"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    issue_468_2,
    "tView.firstCreatePass ?
      getOrCreateTNode(tView, lView[T_HOST], index, TNodeType.Element, null, null) :
      tView.data[adjustedIndex] as TElementNode",
    "tView.firstCreatePass ? getOrCreateTNode(tView, lView[T_HOST], index, TNodeType.Element, \
     null, null) : tView.data[adjustedIndex];"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    issue_468_3,
    "tView.firstCreatePass ?
      getOrCreateTNode() : tView.data[adjustedIndex] as TElementNode",
    "tView.firstCreatePass ? getOrCreateTNode() : tView.data[adjustedIndex];"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    issue_468_4,
    "a ? b : c",
    "a ? b : c"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    issue_468_5,
    "a ? b : c as T",
    "a ? b : c"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    issue_468_6,
    "a.b ? c() : d.e[f] as T",
    "a.b ? c() : d.e[f];"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    issue_468_7,
    "tView.firstCreatePass ? getOrCreateTNode() : tView.data[adjustedIndex]",
    "tView.firstCreatePass ? getOrCreateTNode() : tView.data[adjustedIndex];"
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    enum_simple,
    "enum Foo{ a }",
    "
var Foo;
(function (Foo) {
    Foo[Foo['a'] = 0] = 'a';
})(Foo || (Foo = {}));",
    ok_if_code_eq
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    enum_str,
    "enum State {
  closed = 'closed',
  opened = 'opened',
  mounted = 'mounted',
  unmounted = 'unmounted',
}",
    "var State;
(function(State) {
    State[State['closed'] = 0] = 'closed';
    State[State['opened'] = 1] = 'opened';
    State[State['mounted'] = 2] = 'mounted';
    State[State['unmounted'] = 3] = 'unmounted';
})(State || (State = {
}));
",
    ok_if_code_eq
);

test!(
    ::swc_ecma_parser::Syntax::Typescript(Default::default()),
    |_| strip(),
    enum_export_str,
    "export enum State {
  closed = 'closed',
  opened = 'opened',
  mounted = 'mounted',
  unmounted = 'unmounted',
}",
    "export var State;
(function(State) {
    State[State['closed'] = 0] = 'closed';
    State[State['opened'] = 1] = 'opened';
    State[State['mounted'] = 2] = 'mounted';
    State[State['unmounted'] = 3] = 'unmounted';
})(State || (State = {
}));",
    ok_if_code_eq
);
