//! The update command's refusals before it opens any serial port.

use super::*;

#[test]
fn the_cli_refuses_an_unpinned_helper_before_any_serial_access() {
    let checkout = tempfile::tempdir().unwrap();
    fs::create_dir(checkout.path().join("private")).unwrap();

    let installed = image(30000, 3);
    let unit = unit(development::payload(&installed, &BANK1).unwrap());
    write(
        &checkout.path().join("private/baseline.bin"),
        unit.baseline.bytes(),
    );

    let opened = Rc::new(RefCell::new(0));
    let count = opened.clone();
    let context = Context {
        root: Root::new(checkout.path()).unwrap(),
        tool_sha256: sha256(b"test binary"),
        open: Box::new(move |_| {
            *count.borrow_mut() += 1;
            Ok(Box::new(Fake::new(vec![0xff; CAPACITY])))
        }),
        pin: Pin::Synthetic,
    };

    let private = |name: &str| checkout.path().join("private").join(name);
    let mut helper = vec![0u8; protocol::HELPER_BYTES];
    helper[..4].copy_from_slice(&(protocol::HELPER_ADDRESS + 0x21).to_le_bytes());
    write(&private("helper.bin"), &helper);

    let evidence = |output: &str| crate::writers::update::Evidence {
        port: "/dev/fake".into(),
        baseline: private("baseline.bin"),
        rom_report: private("rom/report.json"),
        small_report: private("small/report.json"),
        helper: Some(private("helper.bin")),
        output: private(output),
        inspection: None,
        plan: None,
        expected_plan_sha256: None,
    };

    let review = |name: &str, image: &[u8]| -> PathBuf {
        let payload = development::payload(image, &BANK1).unwrap();
        write(&private(name).join("image.bin"), image);
        write(
            &private(name).join("review.json"),
            json!({"schema_version": 1, "kind": "plaf103_development_image_review",
                   "image_kind": "stock", "ota_sha256": sha256(image),
                   "payload_sha256": sha256(payload), "payload_bytes": payload.len(),
                   "manifest_sha256": null, "review": {"stock_sha256": sha256(image)},
                   "reviewer_sha256": "0".repeat(64), "application_boot_verified": false})
            .to_string()
            .as_bytes(),
        );
        private(name)
    };

    let args = crate::writers::update::Args {
        mode: crate::writers::update::Mode::Inspect,
        evidence: evidence("inspect"),
        target: review("target", &variant(&installed, false)),
        source: Some(review("source", &installed)),
        original_read: None,
        bank: 1,
        restore_marker: false,
        interrupted: false,
        stopped_session: None,
    };

    // A synthetic image is not the pinned stock image.
    let error = crate::writers::update::run(&context, &args).err().unwrap();
    assert!(
        matches!(
            error,
            crate::Error::Image(wellfed_image::Error::Review(
                wellfed_image::review::ReviewError::NotOriginalStock
            ))
        ),
        "{error}"
    );
    assert_eq!(error.kind(), crate::error::ErrorKind::Refused);
    assert_eq!(*opened.borrow(), 0);
    assert!(!private("inspect").exists());
    let _ = evidence("unused");
}

/// An execution runs only the plan its reviewer passed. A missing plan or
/// hash, a hash of anything else, a plan edited after it was prepared, and a
/// plan another binary prepared are each refused before any serial access.
#[test]
fn an_execution_refuses_every_plan_but_the_reviewed_one() {
    use crate::evidence::session::{SessionError, reviewed_plan, write_plan};
    use wellfed_formats::flash_plan::{Kind, Reviewed, ToolHashes, UpdateBindings};

    let (checkout, root) = private_root();
    let source = image(30000, 3);
    let old = development::payload(&source, &BANK1).unwrap().to_vec();
    let unit = unit(&old);
    let current = &unit.flash[START..START + old.len()];
    let plan = development::plan(&update(Some(&source), &source, BANK1), current, None).unwrap();
    let reviewed = |tool: &str| Reviewed {
        bindings: UpdateBindings {
            schema_version: 1,
            kind: Kind::Bank1Update,
            port: "/dev/ttyUSB0".into(),
            interrupted: false,
            original_read_sha256: None,
            restore_marker: false,
            source_review_sha256: Some(sha256(b"source")),
            target_review_sha256: sha256(b"target"),
            baseline_sha256: sha256(b"baseline"),
            rom_report_sha256: sha256(b"rom"),
            bounded_read_report_sha256: sha256(b"small"),
            tool_sha256: ToolHashes {
                wellfed_flash: sha256(tool.as_bytes()),
            },
            stopped_plan_sha256: None,
        },
        inspection_report_sha256: sha256(b"inspection"),
        operation_plan: plan.clone(),
    };
    let expected = reviewed("this binary");
    let path = checkout.path().join("private/plan.json");
    let sha = write_plan(&root, &path, &expected).unwrap();

    let raw = reviewed_plan(&root, Some(&path), Some(&sha), &expected).unwrap();
    assert_eq!(raw, fs::read(&path).unwrap());

    let refused = |path: Option<&Path>, sha: Option<&str>, expected| match reviewed_plan(
        &root, path, sha, expected,
    )
    .err()
    {
        Some(crate::Error::Session(error)) => Some(error),
        other => panic!("{other:?}"),
    };
    for (path, sha) in [(None, Some(sha.as_str())), (Some(path.as_path()), None)] {
        assert!(matches!(
            refused(path, sha, &expected),
            Some(SessionError::ReviewedPlanMissing)
        ));
    }
    assert!(matches!(
        refused(Some(&path), Some(&sha256(b"another plan")), &expected),
        Some(SessionError::ReviewedPlanDiffers)
    ));
    let other_binary = reviewed("another binary");
    assert!(matches!(
        refused(Some(&path), Some(&sha), &other_binary),
        Some(SessionError::ReviewedPlanDiffers)
    ));

    // An edit the reviewer hashed afterwards still differs from the plan
    // this execution recomputes.
    for (field, value) in [
        ("port", json!("/dev/ttyUSB1")),
        ("inspection_report_sha256", json!(sha256(b"other"))),
        ("operation_plan", {
            let mut edited = serde_json::to_value(&plan).unwrap();
            edited["erase_sectors"] = json!(plan.erase_sectors + 1);
            edited
        }),
    ] {
        let mut edited: Value = serde_json::from_slice(&raw).unwrap();
        edited[field] = value;
        let path = checkout.path().join(format!("private/{field}.json"));
        let bytes = edited.to_string().into_bytes();
        write(&path, &bytes);
        assert!(
            matches!(
                refused(Some(&path), Some(&sha256(&bytes)), &expected),
                Some(SessionError::ReviewedPlanDiffers)
            ),
            "{field}"
        );
    }
}
