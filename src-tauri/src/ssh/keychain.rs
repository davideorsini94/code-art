//! Password nel Portachiavi di macOS: elementi "password generica" di un servizio (`Code Art SSH` per i server SSH,
//! `Code Art DB` per i database), con account = id del server o della connessione. Unico posto in cui stanno i segreti (mai SQLite, file, log, eventi). I messaggi d'errore
//! riportano solo l'operazione e il codice `OSStatus`, mai il segreto. Tutto il codice `unsafe` sta in `sys`.

/// Servizio delle password dei server SSH.
pub const SERVICE: &str = "Code Art SSH";
/// Servizio delle password delle connessioni ai database.
pub const DB_SERVICE: &str = "Code Art DB";

const ERR_SEC_SUCCESS: i32 = 0;
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
/// Dato restituito dal Portachiavi non decodificabile (tipo inatteso o non UTF-8).
const ERR_SEC_DECODE: i32 = -26275;

fn status_error(op: &str, status: i32) -> String {
    let why = match status {
        -128 => "operazione annullata",
        -25293 => "accesso negato",
        -25308 => "Portachiavi bloccato o non accessibile",
        ERR_SEC_DECODE => "dato non leggibile",
        _ => "errore del Portachiavi",
    };
    format!("Portachiavi: impossibile {op} la password ({why}, codice {status})")
}

fn check_account(account: &str) -> Result<(), String> {
    if account.is_empty() {
        return Err("Portachiavi: account mancante".into());
    }
    Ok(())
}

/// Salva (o sostituisce) la password di `account`.
pub fn set(service: &str, account: &str, secret: &str) -> Result<(), String> {
    check_account(account)?;
    match sys::update(service, account, secret.as_bytes()) {
        ERR_SEC_SUCCESS => Ok(()),
        ERR_SEC_ITEM_NOT_FOUND => match sys::add(service, account, secret.as_bytes()) {
            ERR_SEC_SUCCESS => Ok(()),
            // creato nel frattempo da un altro processo: si riprova l'aggiornamento
            ERR_SEC_DUPLICATE_ITEM => match sys::update(service, account, secret.as_bytes()) {
                ERR_SEC_SUCCESS => Ok(()),
                s => Err(status_error("salvare", s)),
            },
            s => Err(status_error("salvare", s)),
        },
        s => Err(status_error("salvare", s)),
    }
}

/// Password di `account`, `None` se non c'è.
pub fn get(service: &str, account: &str) -> Result<Option<String>, String> {
    check_account(account)?;
    match sys::copy_data(service, account) {
        Ok(None) => Ok(None),
        Ok(Some(bytes)) => String::from_utf8(bytes).map(Some).map_err(|_| status_error("leggere", ERR_SEC_DECODE)),
        Err(s) => Err(status_error("leggere", s)),
    }
}

/// `true` se `account` ha una password; non legge il segreto (nessuna richiesta di accesso al dato).
pub fn exists(service: &str, account: &str) -> Result<bool, String> {
    check_account(account)?;
    match sys::exists(service, account) {
        ERR_SEC_SUCCESS => Ok(true),
        ERR_SEC_ITEM_NOT_FOUND => Ok(false),
        s => Err(status_error("controllare", s)),
    }
}

/// Elimina la password di `account`; assente = già fatto.
pub fn delete(service: &str, account: &str) -> Result<(), String> {
    check_account(account)?;
    match sys::delete(service, account) {
        ERR_SEC_SUCCESS | ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        s => Err(status_error("eliminare", s)),
    }
}

#[cfg(target_os = "macos")]
mod sys {
    //! FFI minima verso Security.framework (`SecItem*`), dizionari costruiti con `core-foundation`.
    use super::{ERR_SEC_DECODE, ERR_SEC_ITEM_NOT_FOUND, ERR_SEC_SUCCESS};
    use core_foundation::base::{CFType, CFTypeRef, OSStatus, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::data::CFData;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};
    use std::ptr;

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        static kSecClass: CFStringRef;
        static kSecClassGenericPassword: CFStringRef;
        static kSecAttrService: CFStringRef;
        static kSecAttrAccount: CFStringRef;
        static kSecValueData: CFStringRef;
        static kSecReturnData: CFStringRef;
        static kSecMatchLimit: CFStringRef;
        static kSecMatchLimitOne: CFStringRef;

        fn SecItemAdd(attributes: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
        fn SecItemUpdate(query: CFDictionaryRef, attributes_to_update: CFDictionaryRef) -> OSStatus;
        fn SecItemCopyMatching(query: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
        fn SecItemDelete(query: CFDictionaryRef) -> OSStatus;
    }

    type Dict = CFDictionary<CFType, CFType>;

    /// Una costante `CFStringRef` di Security.framework come `CFType`.
    fn konst(k: CFStringRef) -> CFType {
        // SAFETY: `k` è una delle costanti esportate da Security.framework, non nulle e valide per tutta la vita del
        // processo; la regola "get" aggiunge un retain che il `CFString` rilascia quando viene distrutto.
        unsafe { CFString::wrap_under_get_rule(k) }.into_CFType()
    }

    /// Classe, servizio e account: identificano l'elemento.
    fn base(service: &str, account: &str) -> Vec<(CFType, CFType)> {
        // SAFETY (per le letture delle costanti `extern`): sono simboli di sola lettura di Security.framework,
        // inizializzati dal loader prima di `main`.
        let (class, generic, k_service, acct) =
            unsafe { (kSecClass, kSecClassGenericPassword, kSecAttrService, kSecAttrAccount) };
        vec![
            (konst(class), konst(generic)),
            (konst(k_service), CFString::new(service).into_CFType()),
            (konst(acct), CFString::new(account).into_CFType()),
        ]
    }

    fn value(secret: &[u8]) -> (CFType, CFType) {
        // SAFETY: vedi `base`.
        let k = unsafe { kSecValueData };
        (konst(k), CFData::from_buffer(secret).into_CFType())
    }

    fn match_one(pairs: &mut Vec<(CFType, CFType)>) {
        // SAFETY: vedi `base`.
        let (limit, one) = unsafe { (kSecMatchLimit, kSecMatchLimitOne) };
        pairs.push((konst(limit), konst(one)));
    }

    pub fn add(service: &str, account: &str, secret: &[u8]) -> OSStatus {
        let mut pairs = base(service, account);
        pairs.push(value(secret));
        let attrs = Dict::from_CFType_pairs(&pairs);
        // SAFETY: `attrs` è un CFDictionary valido per tutta la chiamata; `result` nullo è ammesso (non serve
        // l'elemento creato) e quindi non c'è nulla da rilasciare.
        unsafe { SecItemAdd(attrs.as_concrete_TypeRef(), ptr::null_mut()) }
    }

    pub fn update(service: &str, account: &str, secret: &[u8]) -> OSStatus {
        let query = Dict::from_CFType_pairs(&base(service, account));
        let changes = Dict::from_CFType_pairs(&[value(secret)]);
        // SAFETY: entrambi i dizionari sono validi per tutta la chiamata, che non ne prende possesso.
        unsafe { SecItemUpdate(query.as_concrete_TypeRef(), changes.as_concrete_TypeRef()) }
    }

    pub fn exists(service: &str, account: &str) -> OSStatus {
        let mut pairs = base(service, account);
        match_one(&mut pairs);
        let query = Dict::from_CFType_pairs(&pairs);
        // SAFETY: nessun `kSecReturn*` nella richiesta e `result` nullo: la funzione dice solo se l'elemento c'è,
        // senza restituire oggetti da rilasciare.
        unsafe { SecItemCopyMatching(query.as_concrete_TypeRef(), ptr::null_mut()) }
    }

    pub fn copy_data(service: &str, account: &str) -> Result<Option<Vec<u8>>, OSStatus> {
        let mut pairs = base(service, account);
        match_one(&mut pairs);
        // SAFETY: vedi `base`.
        let ret = unsafe { kSecReturnData };
        pairs.push((konst(ret), CFBoolean::true_value().into_CFType()));
        let query = Dict::from_CFType_pairs(&pairs);
        let mut out: CFTypeRef = ptr::null();
        // SAFETY: `query` è valido per la chiamata; `out` punta a una variabile locale che riceve un riferimento
        // +1 (regola "copy") o resta nullo.
        let status = unsafe { SecItemCopyMatching(query.as_concrete_TypeRef(), &mut out) };
        match status {
            ERR_SEC_SUCCESS if !out.is_null() => {
                // SAFETY: `out` è non nullo e posseduto da noi (+1): il `CFType` lo rilascia una sola volta.
                let item = unsafe { CFType::wrap_under_create_rule(out) };
                item.downcast_into::<CFData>().map(|d| Some(d.bytes().to_vec())).ok_or(ERR_SEC_DECODE)
            }
            ERR_SEC_SUCCESS => Err(ERR_SEC_DECODE),
            ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            s => Err(s),
        }
    }

    pub fn delete(service: &str, account: &str) -> OSStatus {
        let query = Dict::from_CFType_pairs(&base(service, account));
        // SAFETY: `query` è valido per tutta la chiamata, che non ne prende possesso.
        unsafe { SecItemDelete(query.as_concrete_TypeRef()) }
    }
}

#[cfg(not(target_os = "macos"))]
mod sys {
    //! Fuori da macOS non c'è Portachiavi: nessuna password salvata, ogni scrittura fallisce.
    use super::ERR_SEC_ITEM_NOT_FOUND;
    /// errSecUnimplemented
    const UNIMPLEMENTED: i32 = -4;
    pub fn add(_: &str, _: &str, _: &[u8]) -> i32 {
        UNIMPLEMENTED
    }
    pub fn update(_: &str, _: &str, _: &[u8]) -> i32 {
        ERR_SEC_ITEM_NOT_FOUND
    }
    pub fn exists(_: &str, _: &str) -> i32 {
        ERR_SEC_ITEM_NOT_FOUND
    }
    pub fn copy_data(_: &str, _: &str) -> Result<Option<Vec<u8>>, i32> {
        Ok(None)
    }
    pub fn delete(_: &str, _: &str) -> i32 {
        ERR_SEC_ITEM_NOT_FOUND
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_are_fixed_and_distinct() {
        assert_eq!(SERVICE, "Code Art SSH");
        assert_eq!(DB_SERVICE, "Code Art DB");
    }

    #[test]
    fn errors_name_the_operation_and_code_only() {
        let e = status_error("salvare", -25308);
        assert_eq!(e, "Portachiavi: impossibile salvare la password (Portachiavi bloccato o non accessibile, codice -25308)");
        assert!(status_error("leggere", -1).contains("codice -1"));
        assert!(set(SERVICE, "", "x").is_err());
        assert!(get(SERVICE, "").is_err());
        assert!(exists(SERVICE, "").is_err());
        assert!(delete(SERVICE, "").is_err());
    }

    /// Usa il Portachiavi vero: serve il Portachiavi di login sbloccato (sessione utente di macOS, non CI né sandbox
    /// senza accesso al Portachiavi), e il primo accesso può chiedere conferma. Si esegue a mano con
    /// `cargo test -- --ignored keychain_round_trip`. Account casuale, eliminato alla fine.
    #[test]
    #[ignore = "richiede il Portachiavi di login sbloccato"]
    fn keychain_round_trip() {
        let account = format!("codeart-test-{}", uuid::Uuid::new_v4());
        assert_eq!(get(SERVICE, &account).unwrap(), None);
        assert!(!exists(SERVICE, &account).unwrap());
        set(SERVICE, &account, "prima-è").unwrap();
        assert!(exists(SERVICE, &account).unwrap());
        assert_eq!(get(SERVICE, &account).unwrap().as_deref(), Some("prima-è"));
        set(SERVICE, &account, "seconda").unwrap();
        assert_eq!(get(SERVICE, &account).unwrap().as_deref(), Some("seconda"));
        delete(SERVICE, &account).unwrap();
        assert_eq!(get(SERVICE, &account).unwrap(), None);
        assert!(!exists(SERVICE, &account).unwrap());
        delete(SERVICE, &account).unwrap();
    }
}
