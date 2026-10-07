```rust
use super::*;
use crate::service::invoice::{
    create_invoice, finalize_draft_invoice, get_invoice_status, list_invoices, update_invoice,
};
use agento_core::model::{
    BatchItemKind, BatchItemType, BillingInfo, BillingType, ClientError, Invoice, InvoiceId,
    InvoiceStatus, InvoiceType, OfferId, OrganizationId,
};
use agento_core::{db::Db, model::AccountId};
use assert_matches::assert_matches;
use chrono::{Duration, Utc};
use pretty_assertions::assert_eq;
use serde_json::json;
use stripe::{api::ApiResource, Customer, Subscription};

#[derive(Debug)]
enum MockStripeBehavior {
    CreateCustomerSuccess(Customer),
    CreateCustomerFailure(String),
    ListPaymentMethodsSuccess(Vec<stripe::PaymentMethod>),
    ListPaymentMethodsFailure(String),
    AttachPaymentMethodSuccess,
    AttachPaymentMethodFailure(String),
    UpdateCustomerDefaultPaymentMethodSuccess,
    UpdateCustomerDefaultPaymentMethodFailure(String),
    CreateCheckoutSessionSuccess(String),
    CreateCheckoutSessionFailure(String),
    RetrieveSubscriptionSuccess(Subscription),
    RetrieveSubscriptionFailure(String),
    UpdateSubscriptionSuccess(Subscription),
    UpdateSubscriptionFailure(String),
    CancelSubscriptionSuccess,
    CancelSubscriptionFailure(String),
    ListInvoicesSuccess(Vec<stripe::Invoice>),
    ListInvoicesFailure(String),
    CreateInvoiceSuccess(stripe::Invoice),
    CreateInvoiceFailure(String),
    FinalizeInvoiceSuccess(stripe::Invoice),
    FinalizeInvoiceFailure(String),
    CreateInvoiceItemSuccess(stripe::InvoiceItem),
    CreateInvoiceItemFailure(String),
    RetrieveInvoiceSuccess(stripe::Invoice),
    RetrieveInvoiceFailure(String),
}

type BoxedFn<T> = Box<dyn Fn(&mut T)>;

#[derive(Default)]
struct MockStripeClient {
    behaviors: tokio::sync::Mutex<Vec<(String, BoxedFn<MockStripeBehavior>)>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl MockStripeClient {
    fn new() -> Self {
        Self::default()
    }

    fn add_behavior<F>(&self, method: &str, behavior: F)
    where
        F: Fn(&mut MockStripeBehavior) + 'static,
    {
        let boxed: BoxedFn<MockStripeBehavior> = Box::new(behavior);
        self.behaviors
            .try_lock()
            .expect("should be able to lock")
            .push((method.to_string(), boxed));
    }

    async fn handle_request(&self, method: &str) -> Result<MockStripeBehavior, String> {
        let mut behaviors = self.behaviors.try_lock().expect("should be able to lock");
        if let Some((_, fn_box)) = behaviors.iter().next() {
            let (method_name, fn_box) = behaviors.remove(0);
            drop(behaviors);

            if method_name == method {
                let mut behavior = MockStripeBehavior::default();
                fn_box(&mut behavior);
                return Ok(behavior);
            } else {
                return Err(format!(
                    "Expected method {} but got {}",
                    method_name, method
                ));
            }
        }
        Err("No more behaviors".to_string())
    }
}

impl Clone for MockStripeClient {
    fn clone(&self) -> Self {
        Self {
            behaviors: tokio::sync::Mutex::new(
                self.behaviors.try_lock().unwrap().clone(),
            ),
            next_id: std::sync::atomic::AtomicU64::new(
                self.next_id.load(std::sync::atomic::Ordering::SeqCst),
            ),
        }
    }
}

#[tokio::test]
async fn test_stripe_client_behavior() {
    let client = MockStripeClient::new();

    client.add_behavior("GET /customer", |behavior| {
        *behavior = MockStripeBehavior::CreateCustomerSuccess(Customer {
            id: "cus_123".to_string(),
            ..Default::default()
        });
    });

    let result = client.handle_request("GET /customer").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_stripe_client_no_behavior() {
    let client = MockStripeClient::new();

    let result = client.handle_request("GET /customer").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_stripe_client_wrong_method() {
    let client = MockStripeClient::new();

    client.add_behavior("GET /customer", |behavior| {
        *behavior = MockStripeBehavior::CreateCustomerSuccess(Customer {
            id: "cus_123".to_string(),
            ..Default::default()
        });
    });

    let result = client.handle_request("POST /invoice").await;
    assert!(result.is_err());
}
