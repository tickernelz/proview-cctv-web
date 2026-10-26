use rocket::figment::Figment;

pub fn build(figment: Figment) -> rocket::Rocket<rocket::Build> {
    rocket::custom(figment)
        .manage(crate::config::Config::from_env())
        .manage(crate::session::Store::new())
        .mount("/", crate::routes::files())
        .mount("/api", crate::routes::api())
}
