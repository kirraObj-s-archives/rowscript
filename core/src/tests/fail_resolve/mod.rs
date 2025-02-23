use crate::tests::run_err;
use crate::theory::Loc;
use crate::Error;

#[test]
fn test_resolve() {
    match run_err(module_path!()) {
        Error::UnresolvedVar(Loc { line, col, .. }) => {
            assert_eq!(line, 7);
            assert_eq!(col, 9);
        }
        _ => assert!(false),
    }
}
