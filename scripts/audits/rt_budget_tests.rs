// Run without building the plugin:
// rustc --edition=2024 --test scripts/audits/rt_budget_tests.rs -o /tmp/kurv-rt-budget-tests
// /tmp/kurv-rt-budget-tests
#[path = "../../src/voices/internal_rt_budget.rs"]
mod budget;
