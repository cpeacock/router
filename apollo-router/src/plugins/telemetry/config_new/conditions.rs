use opentelemetry::Value;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::plugins::telemetry::config::AttributeValue;
use crate::plugins::telemetry::config_new::Selector;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum Condition<T> {
    /// A condition to check a selection against a value.
    Eq([SelectorOrValue<T>; 2]),
    /// All sub-conditions must be true.
    All(Vec<Condition<T>>),
    /// At least one sub-conditions must be true.
    Any(Vec<Condition<T>>),
    /// The sub-condition must not be true
    Not(Box<Condition<T>>),
    /// Static true condition
    True,
    /// Static false condition
    False,
}

impl<T> Default for Condition<T> {
    fn default() -> Self {
        Self::True
    }
}

impl Condition<()> {
    pub(crate) fn empty<T>() -> Condition<T> {
        Condition::True
    }
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields, rename_all = "snake_case", untagged)]
pub(crate) enum SelectorOrValue<T> {
    /// A constant value.
    Value(AttributeValue),
    /// Selector to extract a value from the pipeline.
    Selector(T),
}

#[allow(dead_code)]
impl<T> Condition<T>
where
    T: Selector,
{
    fn evaluate(&self, request: &T::Request, response: &T::Response) -> bool {
        match self {
            Condition::Eq(eq) => {
                // We don't know if the selection was for the request or result, so we try both.
                let left = eq[0]
                    .on_request(request)
                    .or_else(|| eq[0].on_response(response));
                let right = eq[1]
                    .on_request(request)
                    .or_else(|| eq[1].on_response(response));
                left == right
            }
            Condition::All(all) => all.iter().all(|c| c.evaluate(request, response)),
            Condition::Any(any) => any.iter().any(|c| c.evaluate(request, response)),
            Condition::Not(not) => !not.evaluate(request, response),
            Condition::True => true,
            Condition::False => false,
        }
    }

    pub(crate) fn evaluate_request(&mut self, request: &T::Request) -> Option<bool> {
        match self {
            Condition::Eq(eq) => {
                // We don't know if the selection was for the request or result, so we try both.
                match (eq[0].on_request(request), eq[1].on_request(request)) {
                    (None, None) => None,
                    (None, Some(right)) => {
                        eq[1] = SelectorOrValue::Value(right.into());
                        None
                    }
                    (Some(left), None) => {
                        eq[0] = SelectorOrValue::Value(left.into());
                        None
                    }
                    (Some(left), Some(right)) => {
                        if left == right {
                            *self = Condition::True;
                            Some(true)
                        } else {
                            Some(false)
                        }
                    }
                }
            }
            Condition::All(all) => {
                if all.is_empty() {
                    return Some(true);
                }
                let mut response = Some(true);
                for cond in all {
                    match cond.evaluate_request(request) {
                        Some(resp) => {
                            response = response.map(|r| resp && r);
                        }
                        None => {
                            response = None;
                        }
                    }
                }

                response
            }
            Condition::Any(any) => {
                if any.is_empty() {
                    return Some(true);
                }
                let mut response: Option<bool> = Some(false);
                for cond in any {
                    match cond.evaluate_request(request) {
                        Some(resp) => {
                            response = response.map(|r| resp || r);
                        }
                        None => {
                            response = None;
                        }
                    }
                }

                response
            }
            Condition::Not(not) => not.evaluate_request(request).map(|r| !r),
            Condition::True => Some(true),
            Condition::False => Some(false),
        }
    }

    pub(crate) fn evaluate_response(&self, response: &T::Response) -> bool {
        match self {
            Condition::Eq(eq) => {
                let left = eq[0].on_response(response);
                let right = eq[1].on_response(response);
                left == right
            }
            Condition::All(all) => all.iter().all(|c| c.evaluate_response(response)),
            Condition::Any(any) => any.iter().any(|c| c.evaluate_response(response)),
            Condition::Not(not) => !not.evaluate_response(response),
            Condition::True => true,
            Condition::False => false,
        }
    }
}

impl<T> Selector for SelectorOrValue<T>
where
    T: Selector,
{
    type Request = T::Request;
    type Response = T::Response;

    fn on_request(&self, request: &T::Request) -> Option<Value> {
        match self {
            SelectorOrValue::Value(value) => Some(value.clone().into()),
            // TODO return Some(null) ?!
            SelectorOrValue::Selector(selector) => selector.on_request(request),
        }
    }

    fn on_response(&self, response: &T::Response) -> Option<Value> {
        match self {
            SelectorOrValue::Value(value) => Some(value.clone().into()),
            // TODO return Some(null) ?!
            SelectorOrValue::Selector(selector) => selector.on_response(response),
        }
    }
}

#[cfg(test)]
mod test {
    use opentelemetry::Value;

    use crate::plugins::telemetry::config_new::conditions::Condition;
    use crate::plugins::telemetry::config_new::conditions::SelectorOrValue;
    use crate::plugins::telemetry::config_new::Selector;

    struct TestSelector;
    impl Selector for TestSelector {
        type Request = Option<i64>;
        type Response = Option<i64>;

        fn on_request(&self, request: &Self::Request) -> Option<Value> {
            request.map(Value::I64)
        }

        fn on_response(&self, response: &Self::Response) -> Option<Value> {
            response.map(Value::I64)
        }
    }

    #[test]
    fn test_condition_eq() {
        assert!(Condition::<TestSelector>::Eq([
            SelectorOrValue::Value(1i64.into()),
            SelectorOrValue::Value(1i64.into()),
        ])
        .evaluate(&None, &None));
        assert!(!Condition::<TestSelector>::Eq([
            SelectorOrValue::Value(1i64.into()),
            SelectorOrValue::Value(2i64.into()),
        ])
        .evaluate(&None, &None));
    }

    #[test]
    fn test_condition_eq_selector() {
        assert!(Condition::<TestSelector>::Eq([
            SelectorOrValue::Selector(TestSelector),
            SelectorOrValue::Value(1i64.into()),
        ])
        .evaluate(&Some(1i64), &None));
        assert!(Condition::<TestSelector>::Eq([
            SelectorOrValue::Value(1i64.into()),
            SelectorOrValue::Selector(TestSelector),
        ])
        .evaluate(&Some(1i64), &None));

        assert!(Condition::<TestSelector>::Eq([
            SelectorOrValue::Selector(TestSelector),
            SelectorOrValue::Value(2i64.into()),
        ])
        .evaluate(&None, &Some(2i64)));
        assert!(Condition::<TestSelector>::Eq([
            SelectorOrValue::Value(2i64.into()),
            SelectorOrValue::Selector(TestSelector),
        ])
        .evaluate(&None, &Some(2i64)));

        assert!(!Condition::<TestSelector>::Eq([
            SelectorOrValue::Selector(TestSelector),
            SelectorOrValue::Value(3i64.into()),
        ])
        .evaluate(&None, &None));
        assert!(!Condition::<TestSelector>::Eq([
            SelectorOrValue::Value(3i64.into()),
            SelectorOrValue::Selector(TestSelector),
        ])
        .evaluate(&None, &None));
    }

    #[test]
    fn test_condition_not() {
        assert!(Condition::<TestSelector>::Not(Box::new(Condition::Eq([
            SelectorOrValue::Value(1i64.into()),
            SelectorOrValue::Value(2i64.into()),
        ])))
        .evaluate(&None, &None));

        assert!(!Condition::<TestSelector>::Not(Box::new(Condition::Eq([
            SelectorOrValue::Value(1i64.into()),
            SelectorOrValue::Value(1i64.into()),
        ])))
        .evaluate(&None, &None));
    }

    #[test]
    fn test_condition_all() {
        assert!(Condition::<TestSelector>::All(vec![
            Condition::Eq([
                SelectorOrValue::Value(1i64.into()),
                SelectorOrValue::Value(1i64.into()),
            ]),
            Condition::Eq([
                SelectorOrValue::Value(2i64.into()),
                SelectorOrValue::Value(2i64.into()),
            ])
        ])
        .evaluate(&None, &None));

        assert!(!Condition::<TestSelector>::All(vec![
            Condition::Eq([
                SelectorOrValue::Value(1i64.into()),
                SelectorOrValue::Value(1i64.into()),
            ]),
            Condition::Eq([
                SelectorOrValue::Value(1i64.into()),
                SelectorOrValue::Value(2i64.into()),
            ])
        ])
        .evaluate(&None, &None));
    }
    #[test]
    fn test_condition_any() {
        assert!(Condition::<TestSelector>::Any(vec![
            Condition::Eq([
                SelectorOrValue::Value(1i64.into()),
                SelectorOrValue::Value(1i64.into()),
            ]),
            Condition::Eq([
                SelectorOrValue::Value(1i64.into()),
                SelectorOrValue::Value(2i64.into()),
            ])
        ])
        .evaluate(&None, &None));

        assert!(!Condition::<TestSelector>::All(vec![
            Condition::Eq([
                SelectorOrValue::Value(1i64.into()),
                SelectorOrValue::Value(2i64.into()),
            ]),
            Condition::Eq([
                SelectorOrValue::Value(1i64.into()),
                SelectorOrValue::Value(2i64.into()),
            ])
        ])
        .evaluate(&None, &None));
    }
}
