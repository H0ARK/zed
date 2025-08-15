use gpui::{IntoElement, ParentElement};
use ui::{List, ListItem, prelude::*};

/// Centralized definitions for Zed AI plans
pub struct PlanDefinitions;

impl PlanDefinitions {
    pub const AI_DESCRIPTION: &'static str = "Zed offers a complete agentic experience, with robust editing and reviewing features to collaborate with AI.";

    pub fn free_plan(&self) -> impl IntoElement {
        List::new()
            .child(ListItem::new("prompts-50").child("50 prompts with Claude models"))
            .child(ListItem::new("edits-2000").child("2,000 accepted edit predictions"))
    }

    pub fn pro_trial(&self, period: bool) -> impl IntoElement {
        List::new()
            .child(ListItem::new("prompts-150").child("150 prompts with Claude models"))
            .child(ListItem::new("edits-unlimited-1")
                .child("Unlimited edit predictions with Zeta, our open-source model")
            )
            .when(period, |this| {
                this.child(ListItem::new("trial-14-days")
                    .child("Try it out for 14 days for free, no credit card required")
                )
            })
    }

    pub fn pro_plan(&self, price: bool) -> impl IntoElement {
        List::new()
            .child(ListItem::new("prompts-500").child("500 prompts with Claude models"))
            .child(ListItem::new("edits-unlimited-2")
                .child("Unlimited edit predictions with Zeta, our open-source model")
            )
            .when(price, |this| {
                this.child(ListItem::new("price-20").child("$20 USD per month"))
            })
    }
}
