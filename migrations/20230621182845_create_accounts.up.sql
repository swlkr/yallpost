create table if not exists accounts (
    id int primary key,
    token text not null,
    name text not null,
    updated_at int not null,
    created_at int not null
);