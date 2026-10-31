#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, atomic::{AtomicI64, Ordering}};
use trainlead_marketplace::{Error, Result, auth::AuthService, clock::Clock, db};

struct FakeClock(AtomicI64);
impl Clock for FakeClock { fn now(&self) -> i64 { self.0.load(Ordering::SeqCst) } }

async fn setup() -> Result<(AuthService, Arc<FakeClock>)> {
    let clock = Arc::new(FakeClock(AtomicI64::new(1_800_000_000)));
    let auth = AuthService::new(db::connect("sqlite::memory:").await?, vec![42;32], clock.clone()).await?;
    Ok((auth,clock))
}

#[tokio::test]
async fn password_login_rotation_reuse_revokes_the_token_family() -> Result<()> {
    let (auth, _) = setup().await?;
    auth.register("trainer@example.com", "a-long-test-password".into(), true).await?;
    let first = auth.login("trainer@example.com", "a-long-test-password".into()).await?;
    assert_eq!(first.expires_in, 900);
    let actor = auth.authenticate(&first.access_token).await?;
    assert!(matches!(auth.require(&actor,"requirement:publish").await,Err(Error::Forbidden)));
    let second = auth.refresh(&first.refresh_token).await?;
    assert!(auth.authenticate(&first.access_token).await.is_err());
    auth.authenticate(&second.access_token).await?;
    assert!(matches!(auth.refresh(&first.refresh_token).await,Err(Error::Unauthorized)));
    assert!(auth.authenticate(&second.access_token).await.is_err());
    assert!(auth.refresh(&second.refresh_token).await.is_err());
    Ok(())
}

#[tokio::test]
async fn expiry_and_revoke_all_invalidate_access_immediately() -> Result<()> {
    let (auth, clock) = setup().await?;
    auth.register("viewer@example.com", "a-long-test-password".into(), true).await?;
    let first = auth.login("viewer@example.com", "a-long-test-password".into()).await?;
    clock.0.fetch_add(900,Ordering::SeqCst);
    assert!(auth.authenticate(&first.access_token).await.is_err());
    let second = auth.login("viewer@example.com", "a-long-test-password".into()).await?;
    let third = auth.login("viewer@example.com", "a-long-test-password".into()).await?;
    let actor = auth.authenticate(&second.access_token).await?;
    auth.logout(&actor,true).await?;
    assert!(auth.authenticate(&third.access_token).await.is_err());
    assert!(auth.refresh(&third.refresh_token).await.is_err());
    Ok(())
}

#[tokio::test]
async fn bootstrap_rbac_and_denials_are_audited_without_contact_fields() -> Result<()> {
    let (auth, _) = setup().await?;
    auth.bootstrap("admin@example.com", "a-long-test-password".into()).await?;
    assert!(matches!(auth.bootstrap("other@example.com", "a-long-test-password".into()).await,Err(Error::Forbidden)));
    let admin_tokens = auth.login("admin@example.com", "a-long-test-password".into()).await?;
    let admin = auth.authenticate(&admin_tokens.access_token).await?;
    auth.register("viewer@example.com", "a-long-test-password".into(),true).await?;
    let tokens = auth.login("viewer@example.com", "a-long-test-password".into()).await?;
    let viewer = auth.authenticate(&tokens.access_token).await?;
    assert!(matches!(auth.users(&viewer,0,10).await,Err(Error::Forbidden)));
    assert!(matches!(auth.assign_role(&viewer,viewer.user_id(),"SuperAdmin").await,Err(Error::Forbidden)));
    assert!(auth.assign_role(&admin,viewer.user_id(),"VendorOwner").await.is_err());
    auth.assign_role(&admin,viewer.user_id(),"Trainer").await?;
    assert!(matches!(auth.require(&viewer,"requirement:publish").await,Err(Error::Forbidden)));
    let entries = auth.audit_entries(&admin,0,100).await?;
    assert!(entries.iter().any(|entry| entry.action=="authorization.denied" && entry.outcome=="denied"));
    let encoded = serde_json::to_string(&auth.users(&admin,0,10).await?)?;
    assert!(!encoded.contains("example.com"));
    Ok(())
}

#[tokio::test]
async fn unknown_accounts_and_wrong_passwords_have_the_same_error_and_lockout() -> Result<()> {
    let (auth, clock) = setup().await?;
    auth.register("viewer@example.com", "a-long-test-password".into(),true).await?;
    for _ in 0..5 {
        assert!(matches!(auth.login("viewer@example.com","wrong-password".into()).await,Err(Error::Unauthorized)));
        assert!(matches!(auth.login("unknown@example.com","wrong-password".into()).await,Err(Error::Unauthorized)));
    }
    assert!(matches!(auth.login("viewer@example.com","a-long-test-password".into()).await,Err(Error::Unauthorized)));
    clock.0.fetch_add(901,Ordering::SeqCst);
    auth.login("viewer@example.com","a-long-test-password".into()).await?;
    assert!(auth.register("viewer@example.com","a-long-test-password".into(),true).await.is_ok());
    assert!(auth.register("other@example.com","a-long-test-password".into(),false).await.is_err());
    Ok(())
}