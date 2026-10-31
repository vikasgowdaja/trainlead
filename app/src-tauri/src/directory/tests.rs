use chrono::NaiveDate;
use trainlead_core::{dedup::DedupThresholds, models::{Message, Mode, Provision, Requirement}, store::Store};

use super::*;

const NOW: i64 = 1_793_500_000;
const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n";

fn temporary() -> PathBuf {
    std::env::temp_dir().join(format!("trainlead-directory-{}", uuid::Uuid::new_v4()))
}

fn open() -> Result<(Directory, PathBuf)> {
    let root = temporary();
    Ok((Directory::open(&root.join("directory.db"), &root.join("documents"))?, root))
}

fn vendor() -> VendorProfile {
    VendorProfile {
        legal_name: "Asha Trainings Pvt Ltd".into(), city: Some("Mysuru".into()), state: Some("Karnataka".into()),
        contact_name: Some("Asha".into()), contact_phone: Some("9845012345".into()),
        domains: vec!["Java".into(), "Angular".into()], ..Default::default()
    }
}

fn trainer() -> TrainerProfile {
    TrainerProfile {
        name: "Ravi Kumar".into(), city: Some("Bengaluru".into()), phone: Some("9900011122".into()),
        experience_years: Some(8), rate_min: Some(6000), rate_max: Some(9000), modes: vec![Mode::Offline, Mode::Online],
        skills: vec![SkillEntry { name: "AngularJS".into(), proficiency: 5, years: Some(6), last_used: Some(2026),
            trainings_delivered: Some(40), evidence_url: Some("https://example.org/cert".into()), verified: true }],
        ..Default::default()
    }
}

fn upload(kind: &str, bytes: &[u8]) -> NewDocument {
    NewDocument { kind: kind.into(), file_name: "C:\\fakepath\\resume.pdf".into(), data_base64: STANDARD.encode(bytes),
        credential_url: None, expires_on: None }
}

#[test]
fn vendor_onboarding_moves_from_draft_through_review_with_reasons() -> Result<()> {
    let (mut directory, root) = open()?;
    let incomplete = VendorProfile { legal_name: "Bala Skills".into(), ..Default::default() };
    let id = directory.save_vendor(None, incomplete, NOW)?;
    let error = directory.submit(Kind::Vendor, id, NOW).err().map(|error| error.to_string()).unwrap_or_default();
    assert!(error.contains("Company profile") && error.contains("Domains served"), "{error}");
    assert!(directory.decide(Kind::Vendor, id, true, None, NOW).is_err(), "drafts cannot be approved");

    directory.save_vendor(Some(id), vendor(), NOW)?;
    directory.submit(Kind::Vendor, id, NOW)?;
    assert!(directory.decide(Kind::Vendor, id, false, Some("  ".into()), NOW).is_err(), "rejection needs a reason");
    directory.decide(Kind::Vendor, id, false, Some("GST certificate missing".into()), NOW)?;
    let rejected = &directory.vendors(&[])?[0];
    assert_eq!((rejected.status, rejected.review_note.as_deref()), (Status::Rejected, Some("GST certificate missing")));

    directory.add_document(Kind::Vendor, id, upload("GST certificate", PDF), NOW)?;
    directory.save_vendor(Some(id), vendor(), NOW)?;
    assert_eq!(directory.vendors(&[])?[0].status, Status::Draft, "editing a rejected profile reopens it");
    directory.submit(Kind::Vendor, id, NOW)?;
    directory.decide(Kind::Vendor, id, true, None, NOW)?;
    let approved = &directory.vendors(&[])?[0];
    assert_eq!(approved.status, Status::Approved);
    assert!(approved.checklist.iter().all(|step| step.done));
    drop(directory);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn trainer_needs_a_resume_and_cannot_self_verify_skills() -> Result<()> {
    let (mut directory, root) = open()?;
    let id = directory.save_trainer(None, trainer(), NOW)?;
    assert!(!directory.trainers()?[0].profile.skills[0].verified, "new profiles cannot claim verified skills");
    let error = directory.submit(Kind::Trainer, id, NOW).err().map(|error| error.to_string()).unwrap_or_default();
    assert!(error.contains("Resume uploaded"), "{error}");

    let mut certificate = upload("Certificate", PDF);
    certificate.expires_on = NaiveDate::from_ymd_opt(2027, 3, 31);
    certificate.credential_url = Some("https://credly.com/badge/1".into());
    let saved = directory.add_document(Kind::Trainer, id, certificate, NOW)?;
    assert_eq!((saved.file_name.as_str(), saved.media_type.as_str()), ("resume.pdf", "application/pdf"));
    directory.add_document(Kind::Trainer, id, upload("Resume", PDF), NOW)?;
    directory.submit(Kind::Trainer, id, NOW)?;

    directory.set_skill_verified(id, "AngularJS", true, NOW)?;
    directory.save_trainer(Some(id), trainer(), NOW)?;
    assert!(directory.trainers()?[0].profile.skills[0].verified, "admin verification survives edits");
    directory.set_skill_verified(id, "AngularJS", false, NOW)?;
    directory.save_trainer(Some(id), trainer(), NOW)?;
    assert!(!directory.trainers()?[0].profile.skills[0].verified, "edits cannot grant verification");
    directory.decide(Kind::Trainer, id, true, None, NOW)?;
    assert_eq!(directory.trainers()?[0].status, Status::Approved);
    drop(directory);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn documents_are_checked_by_content_and_size_and_deleted_with_their_owner() -> Result<()> {
    let (mut directory, root) = open()?;
    let id = directory.save_trainer(None, trainer(), NOW)?;
    assert!(directory.add_document(Kind::Trainer, id, upload("Resume", b"MZ\x90\x00 not a pdf"), NOW).is_err());
    assert!(directory.add_document(Kind::Trainer, id, upload("GST certificate", PDF), NOW).is_err(), "vendor-only type");
    let mut big = b"%PDF-".to_vec();
    big.resize(MAX_DOCUMENT_BYTES + 1, b' ');
    assert!(directory.add_document(Kind::Trainer, id, upload("Resume", &big), NOW).is_err());
    let png = directory.add_document(Kind::Trainer, id, upload("ID proof", b"\x89PNG\r\n\x1a\n...."), NOW)?;
    assert_eq!(png.media_type, "image/png");
    assert_eq!(std::fs::read_dir(root.join("documents"))?.count(), 1);
    directory.delete(Kind::Trainer, id)?;
    assert!(directory.trainers()?.is_empty());
    assert_eq!(std::fs::read_dir(root.join("documents"))?.count(), 0);
    drop(directory);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

fn requirement(vendor: &str, phone: &str) -> Requirement {
    Requirement {
        tech_stack: vec!["Java".into()], start_date: None, end_date: None, duration_days: None, college: None,
        city: Some("Pune".into()), state: None, mode: None, audience: None, travel: Provision::Unknown,
        food: Provision::Unknown, accommodation: Provision::Unknown, logistics_note: None, rate: None,
        payment_date: None, vendor_name: Some(vendor.into()), vendor_company: None, vendor_phone: Some(phone.into()),
        summary: format!("Java training by {vendor}"),
    }
}

#[test]
fn whatsapp_vendors_are_suggested_and_linked_by_phone() -> Result<()> {
    let mut store = Store::in_memory(DedupThresholds::default())?;
    let posts = [("Asha", "919845012345", "Java batch Pune"), ("Asha", "919845012345", "Java FDP Pune college"),
        ("Meena", "918888877777", "Java workshop Pune")];
    let messages: Vec<Message> = posts.iter().map(|(sender, _, text)| Message {
        source_group: "Vendors".into(), sender: Some((*sender).into()), timestamp: "2026-10-31T10:00:00+05:30".into(), raw_text: (*text).into(),
    }).collect();
    let ids = store.record_messages(&messages)?;
    let records: Vec<_> = ids.iter().zip(&messages).zip(posts).map(|((id, message), (vendor, phone, _))| (*id, message.clone(), requirement(vendor, phone))).collect();
    store.save_batch(&ids, &records)?;
    let leads = store.clusters()?;

    let (mut directory, root) = open()?;
    let suggestions = directory.suggestions(&leads)?;
    assert_eq!(suggestions.iter().map(|suggestion| (suggestion.phone.as_str(), suggestion.postings)).collect::<Vec<_>>(),
        vec![("919845012345", 2), ("918888877777", 1)]);
    directory.save_vendor(None, vendor(), NOW)?;
    assert_eq!(directory.vendors(&leads)?[0].postings, 2);
    assert_eq!(directory.suggestions(&leads)?.len(), 1, "added vendors drop out of the suggestions");
    drop(directory);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
