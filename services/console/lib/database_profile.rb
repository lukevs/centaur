class DatabaseProfile
  attr_reader :name

  def initialize(name)
    @name = name.freeze
    freeze
  end

  PARADEDB = new("paradedb")
  POSTGRESQL = new("postgresql")
  BY_NAME = {
    PARADEDB.name => PARADEDB,
    POSTGRESQL.name => POSTGRESQL
  }.freeze

  def self.parse(name)
    BY_NAME.fetch(name) do
      raise ArgumentError,
            "unsupported database profile #{name.inspect}; expected paradedb or postgresql"
    end
  end

  private_class_method :new
end
