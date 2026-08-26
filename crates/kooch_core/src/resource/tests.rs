use super::*;

#[test]
fn insert_and_get() {
    let mut resources = Resources::new();
    resources.insert(42_i32);
    resources.insert("hello".to_string());

    assert_eq!(resources.get::<i32>(), Some(&42));
    assert_eq!(resources.get::<String>(), Some(&"hello".to_string()));
    assert_eq!(resources.get::<f32>(), None);
}

#[test]
fn insert_replaces_existing() {
    let mut resources = Resources::new();
    resources.insert(1_i32);
    let old = resources.insert(2_i32);

    assert_eq!(old, Some(1));
    assert_eq!(resources.get::<i32>(), Some(&2));
}

#[test]
fn get_mut() {
    let mut resources = Resources::new();
    resources.insert(vec![1, 2, 3]);

    if let Some(v) = resources.get_mut::<Vec<i32>>() {
        v.push(4);
    }

    assert_eq!(resources.get::<Vec<i32>>(), Some(&vec![1, 2, 3, 4]));
}

#[test]
fn remove() {
    let mut resources = Resources::new();
    resources.insert(42_i32);

    let removed = resources.remove::<i32>();
    assert_eq!(removed, Some(42));
    assert!(!resources.contains::<i32>());
}

#[test]
fn contains() {
    let mut resources = Resources::new();
    assert!(!resources.contains::<i32>());

    resources.insert(42_i32);
    assert!(resources.contains::<i32>());
}

#[test]
fn len_and_is_empty() {
    let mut resources = Resources::new();
    assert!(resources.is_empty());
    assert_eq!(resources.len(), 0);

    resources.insert(42_i32);
    assert!(!resources.is_empty());
    assert_eq!(resources.len(), 1);

    resources.insert("hello".to_string());
    assert_eq!(resources.len(), 2);
}

#[test]
fn get_ptr_by_id() {
    let mut resources = Resources::new();
    resources.insert(42_i32);

    let ptr = resources.get_ptr_by_id(TypeId::of::<i32>());
    assert!(!ptr.is_null());

    let value = unsafe { &*(ptr as *const i32) };
    assert_eq!(*value, 42);

    // Missing type returns null.
    let null_ptr = resources.get_ptr_by_id(TypeId::of::<f64>());
    assert!(null_ptr.is_null());
}

#[test]
fn get_mut_ptr_by_id() {
    let mut resources = Resources::new();
    resources.insert(10_i32);

    let ptr = resources.get_mut_ptr_by_id(TypeId::of::<i32>());
    assert!(!ptr.is_null());

    unsafe {
        *(ptr as *mut i32) = 99;
    }

    assert_eq!(resources.get::<i32>(), Some(&99));

    // Missing type returns null.
    let null_ptr = resources.get_mut_ptr_by_id(TypeId::of::<f64>());
    assert!(null_ptr.is_null());
}
