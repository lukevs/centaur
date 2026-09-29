require "erb"
require "test_helper"

class DatabaseConfigurationTest < ActiveSupport::TestCase
  test "schema dump follows the configured database profile" do
    assert_equal "schema.rb", database_config(nil).dig("default", "schema_dump")
    assert_equal false, database_config("postgresql").dig("default", "schema_dump")
  end

  private

  def database_config(profile)
    with_env("CENTAUR_DATABASE_PROFILE" => profile) do
      source = Rails.root.join("config/database.yml").read
      YAML.safe_load(ERB.new(source).result, aliases: true)
    end
  end
end
