//! Pure presentation of provider-advertised settings; no guessed model IDs.
use harness_domain::conversation::{SessionConfigOption, SessionUsage};

pub fn selector<'a>(options: &'a [SessionConfigOption], command: &str) -> Option<&'a SessionConfigOption> {
    let category = if command == "/model" { "model" } else { "thought_level" };
    options.iter().find(|option| option.category.as_deref() == Some(category))
}

pub fn list(options: &[SessionConfigOption], command: &str) -> String {
    let Some(option) = selector(options, command) else {
        return format!("No disponible: el servidor ACP no anunció opciones para {command}.\n");
    };
    let mut text = format!("{} (actual: {})\n", option.name, option.current_value);
    for (index, value) in option.options.iter().enumerate() {
        text.push_str(&format!("{}. {} [{}]{}\n", index + 1, value.name, value.value,
            if value.value == option.current_value { " ← actual" } else { "" }));
    }
    text.push_str(&format!("Selecciona con {command} <número o id>.\n"));
    text
}

pub fn selected_value<'a>(option: &'a SessionConfigOption, value: &str) -> Result<&'a str, String> {
    // An exact advertised ID wins even if it happens to be numeric.
    option.options.iter().find(|item| item.value == value)
        .or_else(|| value.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|n| option.options.get(n)))
        .map(|item| item.value.as_str())
        .ok_or_else(|| "Opción inválida: elige un número o id anunciado por el servidor.\n".into())
}

pub fn usage(usage: Option<&SessionUsage>) -> String {
    let Some(usage) = usage else { return "Uso no disponible: el servidor todavía no envió métricas ACP.\n".into(); };
    let mut text = if usage.size == 0 {
        format!("Contexto: {} tokens; capacidad no informada.\n", usage.used)
    } else {
        format!("Contexto: {} / {} tokens ({:.1}%).\n", usage.used, usage.size, usage.used as f64 / usage.size as f64 * 100.0)
    };
    text.push_str("Es ocupación del contexto, no consumo facturado por respuesta.\n");
    if let Some(cost) = &usage.cost {
        text.push_str(&format!("Coste acumulado informado: {} {}.\n", cost.amount, cost.currency));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_domain::conversation::ConfigValue;

    #[test]
    fn only_advertised_models_and_values_are_selectable() {
        let option = SessionConfigOption { id:"server-specific".into(), name:"Modelo".into(), category:Some("model".into()), current_value:"a".into(), options:vec![ConfigValue{value:"a".into(),name:"Modelo A".into()}] };
        assert_eq!(selected_value(&option,"1").unwrap(), "a");
        assert!(selected_value(&option,"inventado").is_err());
        assert!(list(&[option],"/effort").contains("No disponible"));
    }

    #[test]
    fn missing_usage_is_not_zero_and_context_is_not_billing() {
        assert!(usage(None).contains("no disponible"));
        let text = usage(Some(&SessionUsage {used:50,size:100,cost:None}));
        assert!(text.contains("50.0%"));
        assert!(text.contains("no consumo facturado"));
    }
}
