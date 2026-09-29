module DatabaseConfiguration
  PROFILE_ENVIRONMENT_VARIABLE = "CENTAUR_DATABASE_PROFILE"

  module_function

  def profile(environment = ENV)
    value = environment.fetch(
      PROFILE_ENVIRONMENT_VARIABLE,
      DatabaseProfile::PARADEDB.name
    )
    DatabaseProfile.parse(value)
  end
end
